use crate::config::VaultConfig;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use wrfcoin_defi_common::error::{DefiError, DefiResult};
use wrfcoin_defi_common::types::{Address, Amount, AssetId, Timestamp};
use wrfcoin_yield_farming::vault::{
    AutoCompoundingVault, CompoundOutcome, VaultGuardrails, VaultRiskSnapshot,
};
use wrfcoin_yield_farming::FarmManager;

/// Opaque identifier for a managed vault.
pub type VaultId = u64;

/// Summary returned from a `VaultManager::compound_vault` call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompoundResult {
    pub vault_id: VaultId,
    pub harvested_rewards: Amount,
    pub reinvested_rewards: Amount,
    pub performance_fee: Amount,
}

impl CompoundResult {
    fn from_outcome(vault_id: VaultId, outcome: CompoundOutcome) -> Self {
        Self {
            vault_id,
            harvested_rewards: outcome.harvested_rewards,
            reinvested_rewards: outcome.reinvested_rewards,
            performance_fee: outcome.performance_fee,
        }
    }
}

/// A single auto-compounding vault managed by the [`VaultManager`].
///
/// Wraps an [`AutoCompoundingVault`] from the yield-farming crate together with
/// its user-facing [`VaultConfig`] so that config updates can be reflected into
/// the underlying strategy without exposing yield-farming internals.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedVault {
    pub vault_id: VaultId,
    pub config: VaultConfig,
    pub inner: AutoCompoundingVault,
}

impl ManagedVault {
    fn new(
        vault_id: VaultId,
        pool_index: usize,
        vault_address: Address,
        config: VaultConfig,
        guardrails: VaultGuardrails,
    ) -> DefiResult<Self> {
        let strategy = config.into_strategy();
        let inner = AutoCompoundingVault::new(pool_index, vault_address, strategy, guardrails)?;
        Ok(Self {
            vault_id,
            config,
            inner,
        })
    }

    /// Update vault config and apply it to the underlying compounding strategy.
    ///
    /// Does NOT reset `last_compound_time` — the existing interval continues
    /// from the last compound, preventing gaming via rapid config changes.
    pub fn update_config(&mut self, new_config: VaultConfig) -> DefiResult<()> {
        // Validate the new config before applying.
        VaultConfig::new(
            new_config.min_reward_threshold,
            new_config.compound_interval_secs,
            new_config.performance_fee_bps,
        )?;

        let last_compound = self.inner.strategy.last_compound_time;
        let mut new_strategy = new_config.into_strategy();
        new_strategy.last_compound_time = last_compound;

        self.inner.strategy = new_strategy;
        self.config = new_config;
        Ok(())
    }

    /// Whether the vault's compound interval has elapsed and rewards exceed the
    /// threshold.
    pub fn is_compound_due(&self, timestamp: Timestamp) -> bool {
        self.inner.strategy.should_compound(timestamp)
            && self.inner.reward_balance >= self.config.min_reward_threshold
    }
}

/// Central registry that owns a [`FarmManager`] and a set of [`ManagedVault`]
/// instances.
///
/// All vault operations go through the `VaultManager` so that the shared
/// `FarmManager` is never mutably borrowed from two places simultaneously.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultManager {
    pub farm: FarmManager,
    vaults: HashMap<VaultId, ManagedVault>,
    next_vault_id: VaultId,
}

impl VaultManager {
    /// Create a new manager with the given rewards-per-second budget.
    pub fn new(total_rewards_per_second: Amount) -> Self {
        Self {
            farm: FarmManager::new(total_rewards_per_second),
            vaults: HashMap::new(),
            next_vault_id: 1,
        }
    }

    /// Add a new LP-token pool to the underlying farm.
    pub fn add_farm_pool(
        &mut self,
        lp_token: AssetId,
        alloc_point: u64,
        timestamp: Timestamp,
    ) -> usize {
        self.farm.add_pool(lp_token, alloc_point, timestamp)
    }

    /// Register a new vault over `pool_index` with the given config.
    ///
    /// Returns the vault's ID.
    pub fn create_vault(
        &mut self,
        pool_index: usize,
        vault_address: Address,
        config: VaultConfig,
    ) -> DefiResult<VaultId> {
        let vault_id = self.next_vault_id;
        let vault = ManagedVault::new(
            vault_id,
            pool_index,
            vault_address,
            config,
            VaultGuardrails::default(),
        )?;
        self.vaults.insert(vault_id, vault);
        self.next_vault_id += 1;
        Ok(vault_id)
    }

    pub fn vault(&self, vault_id: VaultId) -> Option<&ManagedVault> {
        self.vaults.get(&vault_id)
    }

    pub fn vault_mut(&mut self, vault_id: VaultId) -> Option<&mut ManagedVault> {
        self.vaults.get_mut(&vault_id)
    }

    /// Deposit `assets` into `vault_id` on behalf of `user`.
    ///
    /// Returns the number of vault shares minted.
    pub fn deposit(
        &mut self,
        vault_id: VaultId,
        user: Address,
        assets: Amount,
        timestamp: Timestamp,
    ) -> DefiResult<Amount> {
        let vault = self.vaults.get_mut(&vault_id).ok_or(DefiError::InvalidAmount {
            reason: format!("vault {vault_id} not found"),
        })?;
        vault.inner.deposit(&mut self.farm, user, assets, timestamp)
    }

    /// Redeem `shares` from `vault_id` on behalf of `user`.
    ///
    /// Returns the underlying asset amount withdrawn.
    pub fn withdraw(
        &mut self,
        vault_id: VaultId,
        user: Address,
        shares: Amount,
        timestamp: Timestamp,
    ) -> DefiResult<Amount> {
        let vault = self.vaults.get_mut(&vault_id).ok_or(DefiError::InvalidAmount {
            reason: format!("vault {vault_id} not found"),
        })?;
        vault.inner.withdraw(&mut self.farm, user, shares, timestamp)
    }

    /// Trigger a compound cycle for `vault_id`.
    ///
    /// Harvests pending rewards and reinvests them (minus the performance fee)
    /// back into the farm position.
    pub fn compound_vault(
        &mut self,
        vault_id: VaultId,
        timestamp: Timestamp,
    ) -> DefiResult<CompoundResult> {
        let vault = self.vaults.get_mut(&vault_id).ok_or(DefiError::InvalidAmount {
            reason: format!("vault {vault_id} not found"),
        })?;
        let outcome = vault.inner.compound(&mut self.farm, timestamp)?;
        Ok(CompoundResult::from_outcome(vault_id, outcome))
    }

    /// Compound all vaults whose compound interval has elapsed.
    ///
    /// Skips vaults that are not yet due — they are left untouched so their
    /// reward balances continue to accumulate.
    pub fn compound_due_vaults(
        &mut self,
        timestamp: Timestamp,
    ) -> DefiResult<Vec<CompoundResult>> {
        // Collect vault IDs that are due first to avoid borrow conflicts.
        let due: Vec<VaultId> = self
            .vaults
            .values()
            .filter(|v| v.inner.strategy.should_compound(timestamp))
            .map(|v| v.vault_id)
            .collect();

        let mut results = Vec::with_capacity(due.len());
        for vid in due {
            let result = self.compound_vault(vid, timestamp)?;
            results.push(result);
        }
        Ok(results)
    }

    /// Update the risk snapshot for a vault (governance boost + liquid-staking
    /// exchange rate).
    pub fn set_vault_risk_snapshot(
        &mut self,
        vault_id: VaultId,
        snapshot: VaultRiskSnapshot,
    ) -> DefiResult<()> {
        let vault = self.vaults.get_mut(&vault_id).ok_or(DefiError::InvalidAmount {
            reason: format!("vault {vault_id} not found"),
        })?;
        vault.inner.set_risk_snapshot(snapshot)
    }

    /// Number of currently registered vaults.
    pub fn vault_count(&self) -> usize {
        self.vaults.len()
    }

    /// Total assets under management across all vaults (staked + pending rewards).
    pub fn total_aum(&self) -> DefiResult<Amount> {
        let mut total = Amount::ZERO;
        for vault in self.vaults.values() {
            let aum = vault.inner.total_assets()?;
            total = total.checked_add(aum).ok_or(DefiError::Overflow {
                operation: "VaultManager total_aum".into(),
            })?;
        }
        Ok(total)
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

    fn manager() -> VaultManager {
        let mut mgr = VaultManager::new(Amount::new(100));
        mgr.add_farm_pool(AssetId::new("LP"), 100, ts(0));
        mgr
    }

    fn default_config() -> VaultConfig {
        // Low threshold and short interval for unit-test predictability.
        VaultConfig::new(Amount::new(1), 10, 100).unwrap()
    }

    // ── create_vault ──────────────────────────────────────────────────────────

    #[test]
    fn create_vault_increments_id() {
        let mut mgr = manager();
        let id1 = mgr.create_vault(0, addr(1), default_config()).unwrap();
        let id2 = mgr.create_vault(0, addr(2), default_config()).unwrap();
        assert_eq!(id1, 1);
        assert_eq!(id2, 2);
        assert_eq!(mgr.vault_count(), 2);
    }

    #[test]
    fn vault_lookup_returns_correct_vault() {
        let mut mgr = manager();
        let id = mgr.create_vault(0, addr(10), default_config()).unwrap();
        let vault = mgr.vault(id).unwrap();
        assert_eq!(vault.vault_id, id);
        assert_eq!(vault.inner.pool_index, 0);
    }

    // ── deposit / withdraw ────────────────────────────────────────────────────

    #[test]
    fn deposit_mints_shares_proportional_to_assets() {
        let mut mgr = manager();
        let id = mgr.create_vault(0, addr(99), default_config()).unwrap();

        let shares = mgr.deposit(id, addr(1), Amount::new(1_000), ts(0)).unwrap();
        assert_eq!(shares, Amount::new(1_000));

        let vault = mgr.vault(id).unwrap();
        assert_eq!(vault.inner.total_shares, Amount::new(1_000));
        assert_eq!(vault.inner.staked_balance, Amount::new(1_000));
    }

    #[test]
    fn second_deposit_prices_shares_against_accrued_rewards() {
        let mut mgr = manager();
        let id = mgr.create_vault(0, addr(99), default_config()).unwrap();

        mgr.deposit(id, addr(1), Amount::new(1_000), ts(0)).unwrap();
        // After 10 seconds the farm emits 10 * 100 = 1_000 reward units.
        let shares2 = mgr.deposit(id, addr(2), Amount::new(1_000), ts(10)).unwrap();
        // Total assets = 2_000, total_shares = 1_000 → new user gets 500 shares.
        assert_eq!(shares2, Amount::new(500));
    }

    #[test]
    fn withdraw_returns_correct_assets() {
        let mut mgr = manager();
        let id = mgr.create_vault(0, addr(99), default_config()).unwrap();

        mgr.deposit(id, addr(1), Amount::new(1_000), ts(0)).unwrap();
        let shares = mgr.vault(id).unwrap().inner.share_balance(&addr(1));
        let out = mgr.withdraw(id, addr(1), shares, ts(0)).unwrap();
        assert_eq!(out, Amount::new(1_000));

        let vault = mgr.vault(id).unwrap();
        assert_eq!(vault.inner.total_shares, Amount::ZERO);
    }

    #[test]
    fn withdraw_unknown_vault_errors() {
        let mut mgr = manager();
        let err = mgr.withdraw(42, addr(1), Amount::new(10), ts(0)).unwrap_err();
        assert!(matches!(err, DefiError::InvalidAmount { .. }));
    }

    // ── compound_vault ────────────────────────────────────────────────────────

    #[test]
    fn compound_reinvests_rewards_and_deducts_fee() {
        let mut mgr = manager();
        let id = mgr.create_vault(0, addr(99), default_config()).unwrap();

        mgr.deposit(id, addr(1), Amount::new(1_000), ts(0)).unwrap();
        let result = mgr.compound_vault(id, ts(10)).unwrap();

        assert_eq!(result.vault_id, id);
        assert_eq!(result.harvested_rewards, Amount::new(1_000));
        // 1 % fee → 10 units, net = 990
        assert_eq!(result.performance_fee, Amount::new(10));
        assert_eq!(result.reinvested_rewards, Amount::new(990));

        let vault = mgr.vault(id).unwrap();
        assert_eq!(vault.inner.staked_balance, Amount::new(1_990));
        assert_eq!(vault.inner.accrued_fees, Amount::new(10));
    }

    #[test]
    fn compound_below_threshold_skips_reinvestment() {
        let mut mgr = manager();
        // High threshold: compound never fires with only 1_000 reward units.
        let cfg = VaultConfig::new(Amount::new(5_000), 10, 100).unwrap();
        let id = mgr.create_vault(0, addr(99), cfg).unwrap();

        mgr.deposit(id, addr(1), Amount::new(1_000), ts(0)).unwrap();
        let result = mgr.compound_vault(id, ts(10)).unwrap();

        assert_eq!(result.reinvested_rewards, Amount::ZERO);
        assert_eq!(result.performance_fee, Amount::ZERO);
    }

    #[test]
    fn compound_before_interval_skips_reinvestment() {
        let mut mgr = manager();
        // Interval of 3_600 s; compound at t=5 is too early.
        let cfg = VaultConfig::new(Amount::new(1), 3_600, 100).unwrap();
        let id = mgr.create_vault(0, addr(99), cfg).unwrap();

        mgr.deposit(id, addr(1), Amount::new(1_000), ts(0)).unwrap();
        let result = mgr.compound_vault(id, ts(5)).unwrap();

        assert_eq!(result.reinvested_rewards, Amount::ZERO);
    }

    // ── multi-epoch compounding ────────────────────────────────────────────────

    #[test]
    fn multi_epoch_compounding_grows_staked_balance() {
        let mut mgr = manager();
        let id = mgr.create_vault(0, addr(99), default_config()).unwrap();

        mgr.deposit(id, addr(1), Amount::new(1_000), ts(0)).unwrap();

        // Epoch 1: t=10 → harvest 1_000, fee 10, reinvest 990 → staked = 1_990
        let r1 = mgr.compound_vault(id, ts(10)).unwrap();
        assert_eq!(r1.reinvested_rewards, Amount::new(990));
        assert_eq!(mgr.vault(id).unwrap().inner.staked_balance, Amount::new(1_990));

        // Epoch 2: t=20 → harvest 2_000 (100 rps × 20 blocks but vault deposited 990
        // at t=10 so new position is bigger) — the farm emits 10 * 100 = 1_000 per epoch.
        // After reinvest: staked = 1_990 + 990 = 2_980
        let r2 = mgr.compound_vault(id, ts(20)).unwrap();
        assert_eq!(r2.reinvested_rewards, Amount::new(990));
        assert_eq!(mgr.vault(id).unwrap().inner.staked_balance, Amount::new(2_980));
    }

    #[test]
    fn compound_due_vaults_compounds_multiple_vaults() {
        let mut mgr = VaultManager::new(Amount::new(200));
        mgr.add_farm_pool(AssetId::new("LP"), 100, ts(0));

        let id1 = mgr.create_vault(0, addr(10), default_config()).unwrap();
        let id2 = mgr.create_vault(0, addr(20), default_config()).unwrap();

        mgr.deposit(id1, addr(1), Amount::new(1_000), ts(0)).unwrap();
        mgr.deposit(id2, addr(2), Amount::new(1_000), ts(0)).unwrap();

        let results = mgr.compound_due_vaults(ts(10)).unwrap();
        assert_eq!(results.len(), 2);
        let ids: Vec<VaultId> = results.iter().map(|r| r.vault_id).collect();
        assert!(ids.contains(&id1));
        assert!(ids.contains(&id2));
    }

    #[test]
    fn compound_due_vaults_skips_early_vaults() {
        let mut mgr = manager();
        let short_interval = VaultConfig::new(Amount::new(1), 10, 0).unwrap();
        let long_interval = VaultConfig::new(Amount::new(1), 3_600, 0).unwrap();

        let id_short = mgr.create_vault(0, addr(10), short_interval).unwrap();
        let id_long = mgr.create_vault(0, addr(20), long_interval).unwrap();

        mgr.deposit(id_short, addr(1), Amount::new(500), ts(0)).unwrap();
        mgr.deposit(id_long, addr(2), Amount::new(500), ts(0)).unwrap();

        let results = mgr.compound_due_vaults(ts(10)).unwrap();
        let ids: Vec<VaultId> = results.iter().map(|r| r.vault_id).collect();
        assert!(ids.contains(&id_short));
        assert!(!ids.contains(&id_long));
    }

    // ── update_config ─────────────────────────────────────────────────────────

    #[test]
    fn update_config_applies_new_fee_immediately() {
        let mut mgr = manager();
        let id = mgr.create_vault(0, addr(99), default_config()).unwrap();
        mgr.deposit(id, addr(1), Amount::new(1_000), ts(0)).unwrap();

        let new_cfg = VaultConfig::new(Amount::new(1), 10, 500).unwrap(); // 5 % fee
        mgr.vault_mut(id).unwrap().update_config(new_cfg).unwrap();

        let result = mgr.compound_vault(id, ts(10)).unwrap();
        // 5 % of 1_000 = 50
        assert_eq!(result.performance_fee, Amount::new(50));
        assert_eq!(result.reinvested_rewards, Amount::new(950));
    }

    #[test]
    fn update_config_preserves_last_compound_time() {
        let mut mgr = manager();
        let id = mgr.create_vault(0, addr(99), default_config()).unwrap();
        mgr.deposit(id, addr(1), Amount::new(1_000), ts(0)).unwrap();
        mgr.compound_vault(id, ts(10)).unwrap();

        let new_cfg = VaultConfig::new(Amount::new(1), 10, 200).unwrap();
        mgr.vault_mut(id).unwrap().update_config(new_cfg).unwrap();

        let last = mgr.vault(id).unwrap().inner.strategy.last_compound_time;
        assert_eq!(last, ts(10));
    }

    // ── total_aum ─────────────────────────────────────────────────────────────

    #[test]
    fn total_aum_sums_all_vault_assets() {
        let mut mgr = manager();
        let id1 = mgr.create_vault(0, addr(10), default_config()).unwrap();
        let id2 = mgr.create_vault(0, addr(20), default_config()).unwrap();

        mgr.deposit(id1, addr(1), Amount::new(1_000), ts(0)).unwrap();
        mgr.deposit(id2, addr(2), Amount::new(2_000), ts(0)).unwrap();

        let aum = mgr.total_aum().unwrap();
        // Deposits are reflected immediately in staked_balance.
        assert!(aum >= Amount::new(3_000));
    }

    // ── risk_snapshot ─────────────────────────────────────────────────────────

    #[test]
    fn set_vault_risk_snapshot_within_guardrails_succeeds() {
        let mut mgr = manager();
        let id = mgr.create_vault(0, addr(99), default_config()).unwrap();

        mgr.set_vault_risk_snapshot(
            id,
            VaultRiskSnapshot {
                governance_boost_bps: 11_000,
                external_share_growth_bps: 11_000,
            },
        )
        .unwrap();

        let snap = mgr.vault(id).unwrap().inner.risk_snapshot;
        assert_eq!(snap.governance_boost_bps, 11_000);
    }
}
