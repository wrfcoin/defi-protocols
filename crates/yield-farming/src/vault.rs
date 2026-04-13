use crate::farm::FarmManager;
use crate::strategy::{CompoundingStrategy, YieldStrategy};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use wrfcoin_defi_common::error::{DefiError, DefiResult};
use wrfcoin_defi_common::types::{Address, Amount, Timestamp};

const BPS_SCALE: u16 = 10_000;

/// External reward-shaping inputs the vault must compose with safely.
///
/// This typed snapshot lets the vault model launch-adjacent integrations such
/// as governance-weighted boosts and liquid-staking exchange-rate growth
/// without taking a hard dependency on those crates or contracts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct VaultRiskSnapshot {
    /// Governance-driven reward tilt expressed in basis points.
    pub governance_boost_bps: u16,
    /// External share-price growth, such as liquid-staking exchange-rate lift.
    pub external_share_growth_bps: u16,
}

impl VaultRiskSnapshot {
    pub fn neutral() -> Self {
        Self {
            governance_boost_bps: BPS_SCALE,
            external_share_growth_bps: BPS_SCALE,
        }
    }

    pub fn combined_effect_bps(&self) -> u16 {
        let combined = (self.governance_boost_bps as u128)
            .saturating_mul(self.external_share_growth_bps as u128)
            .saturating_div(BPS_SCALE as u128);
        combined.min(u16::MAX as u128) as u16
    }
}

impl Default for VaultRiskSnapshot {
    fn default() -> Self {
        Self::neutral()
    }
}

/// #43-compatible governance input for vault risk composition.
///
/// The governance router normalizes emissions to a fixed budget and exposes
/// a per-pool multiplier in basis points. The vault consumes only that bounded
/// multiplier signal and never mints additional rewards.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct GovernanceBoostInput {
    pub normalized_multiplier_bps: u16,
}

impl GovernanceBoostInput {
    pub fn new(normalized_multiplier_bps: u16) -> DefiResult<Self> {
        if normalized_multiplier_bps == 0 {
            return Err(DefiError::InvalidAmount {
                reason: "governance multiplier must be non-zero".into(),
            });
        }
        Ok(Self {
            normalized_multiplier_bps,
        })
    }
}

/// #39-compatible liquid-staking exchange-rate input.
///
/// `exchange_rate_bps` is expressed as WRF backing per stWRF share against a
/// neutral `10_000` baseline (1.0x).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiquidStakingInput {
    pub exchange_rate_bps: u16,
}

impl LiquidStakingInput {
    pub fn new(exchange_rate_bps: u16) -> DefiResult<Self> {
        if exchange_rate_bps == 0 {
            return Err(DefiError::InvalidAmount {
                reason: "liquid staking exchange rate must be non-zero".into(),
            });
        }
        Ok(Self { exchange_rate_bps })
    }

    /// Build a staking input from pool totals:
    /// exchange_rate_bps = total_wrf_staked / total_stwrf_supply * 10_000.
    pub fn from_pool_totals(
        total_wrf_staked: Amount,
        total_stwrf_supply: Amount,
    ) -> DefiResult<Self> {
        if total_stwrf_supply.is_zero() {
            return Err(DefiError::InvalidAmount {
                reason: "total_stwrf_supply must be non-zero".into(),
            });
        }

        let rate_bps = total_wrf_staked
            .0
            .checked_mul(BPS_SCALE as u128)
            .and_then(|value| value.checked_div(total_stwrf_supply.0))
            .ok_or(DefiError::Overflow {
                operation: "vault liquid staking exchange rate".into(),
            })?;

        Self::new(rate_bps.min(u16::MAX as u128) as u16)
    }
}

impl VaultRiskSnapshot {
    pub fn from_integrations(
        governance: GovernanceBoostInput,
        liquid_staking: LiquidStakingInput,
    ) -> Self {
        Self {
            governance_boost_bps: governance.normalized_multiplier_bps,
            external_share_growth_bps: liquid_staking.exchange_rate_bps,
        }
    }
}

/// Guardrails that keep the vault from stacking multiple yield surfaces beyond
/// the launch envelope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct VaultGuardrails {
    /// Hard cap for governance-weighted reward boosts.
    pub max_governance_boost_bps: u16,
    /// Hard cap for liquid-staking-style exchange-rate growth.
    pub max_external_share_growth_bps: u16,
    /// Hard cap for the combined effective multiplier.
    pub max_combined_effect_bps: u16,
}

impl Default for VaultGuardrails {
    fn default() -> Self {
        Self {
            max_governance_boost_bps: 12_000,
            max_external_share_growth_bps: 12_000,
            max_combined_effect_bps: 13_000,
        }
    }
}

impl VaultGuardrails {
    pub fn validate(&self) -> DefiResult<()> {
        if self.max_governance_boost_bps < BPS_SCALE
            || self.max_external_share_growth_bps < BPS_SCALE
            || self.max_combined_effect_bps < BPS_SCALE
        {
            return Err(DefiError::InvalidAmount {
                reason: "vault guardrail caps must be >= 10_000 bps".into(),
            });
        }

        if self.max_combined_effect_bps < self.max_governance_boost_bps
            && self.max_combined_effect_bps < self.max_external_share_growth_bps
        {
            return Err(DefiError::InvalidAmount {
                reason: "combined vault cap must allow at least one configured surface".into(),
            });
        }

        Ok(())
    }

    pub fn validate_snapshot(&self, snapshot: &VaultRiskSnapshot) -> DefiResult<()> {
        self.validate()?;

        if snapshot.governance_boost_bps > self.max_governance_boost_bps {
            return Err(DefiError::InvalidAmount {
                reason: format!(
                    "governance boost {} bps exceeds vault cap {} bps",
                    snapshot.governance_boost_bps, self.max_governance_boost_bps
                ),
            });
        }

        if snapshot.external_share_growth_bps > self.max_external_share_growth_bps {
            return Err(DefiError::InvalidAmount {
                reason: format!(
                    "external share growth {} bps exceeds vault cap {} bps",
                    snapshot.external_share_growth_bps, self.max_external_share_growth_bps
                ),
            });
        }

        let combined = snapshot.combined_effect_bps();
        if combined > self.max_combined_effect_bps {
            return Err(DefiError::InvalidAmount {
                reason: format!(
                    "combined vault surface {} bps exceeds cap {} bps",
                    combined, self.max_combined_effect_bps
                ),
            });
        }

        Ok(())
    }
}

/// Summary of one compounding cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompoundOutcome {
    pub harvested_rewards: Amount,
    pub reinvested_rewards: Amount,
    pub performance_fee: Amount,
}

/// Auto-compounding share vault layered on top of a `FarmManager` pool.
///
/// The vault holds a single farm position at `vault_address`, mints internal
/// shares to depositors, harvests farm rewards, and reinvests them according to
/// `CompoundingStrategy`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutoCompoundingVault {
    pub pool_index: usize,
    pub vault_address: Address,
    pub strategy: CompoundingStrategy,
    pub guardrails: VaultGuardrails,
    pub risk_snapshot: VaultRiskSnapshot,
    pub total_shares: Amount,
    pub share_balances: HashMap<Address, Amount>,
    pub staked_balance: Amount,
    pub reward_balance: Amount,
    pub accrued_fees: Amount,
}

impl AutoCompoundingVault {
    pub fn new(
        pool_index: usize,
        vault_address: Address,
        strategy: CompoundingStrategy,
        guardrails: VaultGuardrails,
    ) -> DefiResult<Self> {
        guardrails.validate()?;
        Ok(Self {
            pool_index,
            vault_address,
            strategy,
            guardrails,
            risk_snapshot: VaultRiskSnapshot::default(),
            total_shares: Amount::ZERO,
            share_balances: HashMap::new(),
            staked_balance: Amount::ZERO,
            reward_balance: Amount::ZERO,
            accrued_fees: Amount::ZERO,
        })
    }

    pub fn set_risk_snapshot(&mut self, snapshot: VaultRiskSnapshot) -> DefiResult<()> {
        self.guardrails.validate_snapshot(&snapshot)?;
        self.risk_snapshot = snapshot;
        Ok(())
    }

    /// Update vault risk inputs from explicit #43 governance and #39
    /// liquid-staking integration signals.
    pub fn set_integrations(
        &mut self,
        governance: GovernanceBoostInput,
        liquid_staking: LiquidStakingInput,
    ) -> DefiResult<()> {
        self.set_risk_snapshot(VaultRiskSnapshot::from_integrations(
            governance,
            liquid_staking,
        ))
    }

    pub fn share_balance(&self, user: &Address) -> Amount {
        self.share_balances
            .get(user)
            .copied()
            .unwrap_or(Amount::ZERO)
    }

    pub fn total_assets(&self) -> DefiResult<Amount> {
        self.staked_balance
            .checked_add(self.reward_balance)
            .ok_or(DefiError::Overflow {
                operation: "vault total_assets".into(),
            })
    }

    pub fn preview_deposit(&self, assets: Amount) -> DefiResult<Amount> {
        if assets.is_zero() {
            return Err(DefiError::InvalidAmount {
                reason: "vault deposit amount must be non-zero".into(),
            });
        }

        let total_assets = self.total_assets()?;
        if self.total_shares.is_zero() || total_assets.is_zero() {
            return Ok(assets);
        }

        let shares = assets
            .0
            .checked_mul(self.total_shares.0)
            .and_then(|value| value.checked_div(total_assets.0))
            .map(Amount::new)
            .ok_or(DefiError::Overflow {
                operation: "vault preview_deposit".into(),
            })?;

        if shares.is_zero() {
            return Err(DefiError::InvalidAmount {
                reason: "vault deposit would mint zero shares".into(),
            });
        }

        Ok(shares)
    }

    pub fn preview_redeem(&self, shares: Amount) -> DefiResult<Amount> {
        if shares.is_zero() {
            return Err(DefiError::InvalidAmount {
                reason: "vault redeem amount must be non-zero".into(),
            });
        }
        if shares > self.total_shares {
            return Err(DefiError::InsufficientBalance {
                required: shares,
                available: self.total_shares,
            });
        }

        let total_assets = self.total_assets()?;
        if self.total_shares.is_zero() {
            return Err(DefiError::InvalidAmount {
                reason: "vault has no shares outstanding".into(),
            });
        }

        shares
            .mul_fraction(total_assets.0, self.total_shares.0)
            .ok_or(DefiError::Overflow {
                operation: "vault preview_redeem".into(),
            })
    }

    pub fn sync_rewards(
        &mut self,
        farm: &mut FarmManager,
        timestamp: Timestamp,
    ) -> DefiResult<Amount> {
        if self.staked_balance.is_zero() {
            return Ok(Amount::ZERO);
        }

        // Zero-amount deposit reuses the farm's reward accounting without
        // changing the vault's principal position.
        let harvested =
            farm.deposit(self.vault_address, self.pool_index, Amount::ZERO, timestamp)?;
        self.reward_balance =
            self.reward_balance
                .checked_add(harvested)
                .ok_or(DefiError::Overflow {
                    operation: "vault sync_rewards".into(),
                })?;
        Ok(harvested)
    }

    /// Harvest pending farm rewards into the vault's liquid reward balance
    /// without changing principal or share balances.
    pub fn harvest(&mut self, farm: &mut FarmManager, timestamp: Timestamp) -> DefiResult<Amount> {
        self.sync_rewards(farm, timestamp)
    }

    pub fn deposit(
        &mut self,
        farm: &mut FarmManager,
        user: Address,
        assets: Amount,
        timestamp: Timestamp,
    ) -> DefiResult<Amount> {
        self.sync_rewards(farm, timestamp)?;
        let shares = self.preview_deposit(assets)?;

        let pending = farm.deposit(self.vault_address, self.pool_index, assets, timestamp)?;
        self.reward_balance =
            self.reward_balance
                .checked_add(pending)
                .ok_or(DefiError::Overflow {
                    operation: "vault deposit reward sync".into(),
                })?;
        self.staked_balance =
            self.staked_balance
                .checked_add(assets)
                .ok_or(DefiError::Overflow {
                    operation: "vault deposit staked_balance".into(),
                })?;
        self.total_shares = self
            .total_shares
            .checked_add(shares)
            .ok_or(DefiError::Overflow {
                operation: "vault deposit total_shares".into(),
            })?;

        let entry = self.share_balances.entry(user).or_insert(Amount::ZERO);
        *entry = entry.checked_add(shares).ok_or(DefiError::Overflow {
            operation: "vault deposit share balance".into(),
        })?;

        Ok(shares)
    }

    pub fn withdraw(
        &mut self,
        farm: &mut FarmManager,
        user: Address,
        shares: Amount,
        timestamp: Timestamp,
    ) -> DefiResult<Amount> {
        self.sync_rewards(farm, timestamp)?;

        let user_balance = self
            .share_balances
            .get(&user)
            .copied()
            .unwrap_or(Amount::ZERO);
        if shares > user_balance {
            return Err(DefiError::InsufficientBalance {
                required: shares,
                available: user_balance,
            });
        }

        let assets_out = self.preview_redeem(shares)?;
        let reward_portion = Amount::new(self.reward_balance.0.min(assets_out.0));
        let farm_portion = assets_out
            .checked_sub(reward_portion)
            .ok_or(DefiError::Overflow {
                operation: "vault withdraw farm portion".into(),
            })?;

        if farm_portion > self.staked_balance {
            return Err(DefiError::InsufficientBalance {
                required: farm_portion,
                available: self.staked_balance,
            });
        }

        if !farm_portion.is_zero() {
            let pending =
                farm.withdraw(self.vault_address, self.pool_index, farm_portion, timestamp)?;
            self.reward_balance =
                self.reward_balance
                    .checked_add(pending)
                    .ok_or(DefiError::Overflow {
                        operation: "vault withdraw reward sync".into(),
                    })?;
            self.staked_balance =
                self.staked_balance
                    .checked_sub(farm_portion)
                    .ok_or(DefiError::Overflow {
                        operation: "vault withdraw staked_balance".into(),
                    })?;
        }

        self.reward_balance =
            self.reward_balance
                .checked_sub(reward_portion)
                .ok_or(DefiError::Overflow {
                    operation: "vault withdraw reward balance".into(),
                })?;
        self.total_shares = self
            .total_shares
            .checked_sub(shares)
            .ok_or(DefiError::Overflow {
                operation: "vault withdraw total_shares".into(),
            })?;

        let remaining = user_balance
            .checked_sub(shares)
            .ok_or(DefiError::Overflow {
                operation: "vault withdraw share balance".into(),
            })?;
        if remaining.is_zero() {
            self.share_balances.remove(&user);
        } else {
            self.share_balances.insert(user, remaining);
        }

        Ok(assets_out)
    }

    pub fn compound(
        &mut self,
        farm: &mut FarmManager,
        timestamp: Timestamp,
    ) -> DefiResult<CompoundOutcome> {
        self.guardrails.validate_snapshot(&self.risk_snapshot)?;
        let harvested = self.sync_rewards(farm, timestamp)?;
        let reinvested = self.strategy.calculate_reinvestment(
            self.reward_balance,
            self.staked_balance,
            timestamp,
        )?;

        if reinvested.is_zero() {
            return Ok(CompoundOutcome {
                harvested_rewards: harvested,
                reinvested_rewards: Amount::ZERO,
                performance_fee: Amount::ZERO,
            });
        }

        let (net_rewards, fee) = self.strategy.deduct_fee(self.reward_balance)?;
        if net_rewards != reinvested {
            return Err(DefiError::InvalidAmount {
                reason: "compounding strategy reinvestment mismatch".into(),
            });
        }

        self.reward_balance = Amount::ZERO;
        self.accrued_fees = self
            .accrued_fees
            .checked_add(fee)
            .ok_or(DefiError::Overflow {
                operation: "vault compound accrued_fees".into(),
            })?;

        let pending = farm.deposit(self.vault_address, self.pool_index, net_rewards, timestamp)?;
        self.reward_balance =
            self.reward_balance
                .checked_add(pending)
                .ok_or(DefiError::Overflow {
                    operation: "vault compound post-deposit reward sync".into(),
                })?;
        self.staked_balance =
            self.staked_balance
                .checked_add(net_rewards)
                .ok_or(DefiError::Overflow {
                    operation: "vault compound staked_balance".into(),
                })?;
        self.strategy.last_compound_time = timestamp;

        Ok(CompoundOutcome {
            harvested_rewards: harvested,
            reinvested_rewards: net_rewards,
            performance_fee: fee,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wrfcoin_defi_common::types::AssetId;

    fn addr(n: u8) -> Address {
        let mut bytes = [0u8; 32];
        bytes[0] = n;
        Address::from_bytes(bytes)
    }

    fn ts(secs: u64) -> Timestamp {
        Timestamp::new(secs)
    }

    fn farm_manager() -> FarmManager {
        let mut farm = FarmManager::new(Amount::new(100));
        farm.add_pool(AssetId::new("LP"), 100, ts(0)).unwrap();
        farm
    }

    fn vault(strategy: CompoundingStrategy) -> AutoCompoundingVault {
        AutoCompoundingVault::new(0, addr(99), strategy, VaultGuardrails::default()).unwrap()
    }

    #[test]
    fn deposit_and_withdraw_use_share_accounting() {
        let mut farm = farm_manager();
        let mut vault = vault(CompoundingStrategy::new(Amount::new(1), 1, 0));

        let first_shares = vault
            .deposit(&mut farm, addr(1), Amount::new(1_000), ts(0))
            .unwrap();
        assert_eq!(first_shares, Amount::new(1_000));
        assert_eq!(vault.total_shares, Amount::new(1_000));
        assert_eq!(vault.staked_balance, Amount::new(1_000));

        let second_shares = vault
            .deposit(&mut farm, addr(2), Amount::new(1_000), ts(10))
            .unwrap();

        // User 2 deposits after user 1 has already accrued 1_000 units of rewards,
        // so the new shares are priced against a 2_000 asset base.
        assert_eq!(second_shares, Amount::new(500));
        assert_eq!(vault.share_balance(&addr(1)), Amount::new(1_000));
        assert_eq!(vault.share_balance(&addr(2)), Amount::new(500));
        assert_eq!(vault.reward_balance, Amount::new(1_000));

        let redeemed = vault
            .withdraw(&mut farm, addr(2), second_shares, ts(10))
            .unwrap();
        assert_eq!(redeemed, Amount::new(1_000));
        assert_eq!(vault.share_balance(&addr(2)), Amount::ZERO);
        assert_eq!(vault.total_shares, Amount::new(1_000));
    }

    #[test]
    fn compound_reinvests_rewards_and_grows_share_value() {
        let mut farm = farm_manager();
        let mut vault = vault(CompoundingStrategy::new(Amount::new(1), 10, 100));

        vault
            .deposit(&mut farm, addr(1), Amount::new(1_000), ts(0))
            .unwrap();
        let outcome = vault.compound(&mut farm, ts(10)).unwrap();

        assert_eq!(outcome.harvested_rewards, Amount::new(1_000));
        assert_eq!(outcome.performance_fee, Amount::new(10));
        assert_eq!(outcome.reinvested_rewards, Amount::new(990));
        assert_eq!(vault.staked_balance, Amount::new(1_990));
        assert_eq!(vault.reward_balance, Amount::ZERO);
        assert_eq!(vault.accrued_fees, Amount::new(10));

        let redeemed = vault
            .withdraw(&mut farm, addr(1), vault.share_balance(&addr(1)), ts(10))
            .unwrap();
        assert_eq!(redeemed, Amount::new(1_990));
    }

    #[test]
    fn compound_below_threshold_leaves_rewards_unreinvested() {
        let mut farm = farm_manager();
        let mut vault = vault(CompoundingStrategy::new(Amount::new(2_000), 10, 0));

        vault
            .deposit(&mut farm, addr(1), Amount::new(1_000), ts(0))
            .unwrap();
        let outcome = vault.compound(&mut farm, ts(10)).unwrap();

        assert_eq!(outcome.harvested_rewards, Amount::new(1_000));
        assert_eq!(outcome.reinvested_rewards, Amount::ZERO);
        assert_eq!(vault.staked_balance, Amount::new(1_000));
        assert_eq!(vault.reward_balance, Amount::new(1_000));
    }

    #[test]
    fn guardrail_rejects_excessive_combined_surface() {
        let mut farm = farm_manager();
        let mut vault = vault(CompoundingStrategy::new(Amount::new(1), 10, 0));
        vault
            .deposit(&mut farm, addr(1), Amount::new(1_000), ts(0))
            .unwrap();

        let result = vault.set_risk_snapshot(VaultRiskSnapshot {
            governance_boost_bps: 12_000,
            external_share_growth_bps: 12_000,
        });
        assert!(result.is_err());
    }

    #[test]
    fn harvest_keeps_share_accounting_stable() {
        let mut farm = farm_manager();
        let mut vault = vault(CompoundingStrategy::new(Amount::new(1), 10, 0));

        vault
            .deposit(&mut farm, addr(1), Amount::new(1_000), ts(0))
            .unwrap();
        let harvested = vault.harvest(&mut farm, ts(10)).unwrap();

        assert_eq!(harvested, Amount::new(1_000));
        assert_eq!(vault.total_shares, Amount::new(1_000));
        assert_eq!(vault.staked_balance, Amount::new(1_000));
        assert_eq!(vault.reward_balance, Amount::new(1_000));
        assert_eq!(
            vault.preview_redeem(vault.share_balance(&addr(1))).unwrap(),
            Amount::new(2_000)
        );
    }

    #[test]
    fn high_governance_input_within_cap_preserves_share_accounting_after_compound() {
        let mut farm = farm_manager();
        let mut vault = vault(CompoundingStrategy::new(Amount::new(1), 10, 0));

        vault
            .deposit(&mut farm, addr(1), Amount::new(1_000), ts(0))
            .unwrap();
        vault
            .set_risk_snapshot(VaultRiskSnapshot {
                governance_boost_bps: 11_500,
                external_share_growth_bps: 11_000,
            })
            .unwrap();

        let before = vault.share_balance(&addr(1));
        let outcome = vault.compound(&mut farm, ts(10)).unwrap();

        assert_eq!(outcome.reinvested_rewards, Amount::new(1_000));
        assert_eq!(vault.share_balance(&addr(1)), before);
        assert_eq!(vault.total_shares, Amount::new(1_000));
        assert_eq!(
            vault.preview_redeem(vault.share_balance(&addr(1))).unwrap(),
            Amount::new(2_000)
        );
    }

    #[test]
    fn neutral_snapshot_allows_compounding() {
        let mut farm = farm_manager();
        let mut vault = vault(CompoundingStrategy::new(Amount::new(1), 10, 0));
        vault
            .deposit(&mut farm, addr(1), Amount::new(1_000), ts(0))
            .unwrap();
        vault
            .set_risk_snapshot(VaultRiskSnapshot::neutral())
            .unwrap();

        let outcome = vault.compound(&mut farm, ts(10)).unwrap();
        assert_eq!(outcome.reinvested_rewards, Amount::new(1_000));
        assert_eq!(vault.staked_balance, Amount::new(2_000));
    }

    #[test]
    fn integrations_adapter_sets_snapshot_from_governance_and_liquid_staking() {
        let mut vault = vault(CompoundingStrategy::new(Amount::new(1), 10, 0));
        let governance = GovernanceBoostInput::new(11_200).unwrap();
        let liquid = LiquidStakingInput::new(11_000).unwrap();

        vault.set_integrations(governance, liquid).unwrap();

        assert_eq!(vault.risk_snapshot.governance_boost_bps, 11_200);
        assert_eq!(vault.risk_snapshot.external_share_growth_bps, 11_000);
        assert_eq!(vault.risk_snapshot.combined_effect_bps(), 12_320);
    }

    #[test]
    fn liquid_staking_input_derives_exchange_rate_from_pool_totals() {
        let liquid =
            LiquidStakingInput::from_pool_totals(Amount::new(1_050), Amount::new(1_000)).unwrap();
        assert_eq!(liquid.exchange_rate_bps, 10_500);
    }

    #[test]
    fn liquid_staking_input_rejects_zero_supply() {
        let err =
            LiquidStakingInput::from_pool_totals(Amount::new(1_000), Amount::ZERO).unwrap_err();
        assert!(matches!(err, DefiError::InvalidAmount { .. }));
    }

    #[test]
    fn integrations_adapter_rejects_excessive_combined_surface() {
        let mut farm = farm_manager();
        let mut vault = vault(CompoundingStrategy::new(Amount::new(1), 10, 0));
        vault
            .deposit(&mut farm, addr(1), Amount::new(1_000), ts(0))
            .unwrap();

        let governance = GovernanceBoostInput::new(11_800).unwrap();
        let liquid = LiquidStakingInput::new(11_100).unwrap();
        let err = vault.set_integrations(governance, liquid).unwrap_err();
        assert!(matches!(err, DefiError::InvalidAmount { .. }));
    }
}
