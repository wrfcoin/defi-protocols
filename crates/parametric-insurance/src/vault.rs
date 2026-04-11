//! `InsuranceVault`: ERC-4626-style underwriter capital pool.
//!
//! Underwriters deposit WRF into the vault and receive vault shares proportional to
//! their contribution. Premiums collected are added to the vault's assets.
//! When a payout is triggered the payout amount is deducted from total assets.
//!
//! Share price = total_assets / total_shares (initially 1:1).
//!
//! Risk limits enforce:
//! - Maximum exposure per trigger type (across all active policies).
//! - Geographic diversification (max exposure in any single region).
//! - Minimum capital ratio (free capital / total exposure ≥ min_capital_ratio_bps).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use wrfcoin_defi_common::types::{Address, Amount};

use crate::error::{InsuranceError, InsuranceResult};

/// Vault shares issued to underwriters (analogous to ERC-4626 `shares`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct VaultShares(pub u128);

impl VaultShares {
    pub const ZERO: VaultShares = VaultShares(0);

    pub fn new(shares: u128) -> Self {
        Self(shares)
    }

    pub fn is_zero(&self) -> bool {
        self.0 == 0
    }
}

impl std::fmt::Display for VaultShares {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}shares", self.0)
    }
}

/// Configuration for the vault.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultConfig {
    /// Minimum capital ratio in basis points.
    /// Free capital must be ≥ this fraction of total insured exposure.
    /// E.g., 12_000 bps = 120%.
    pub min_capital_ratio_bps: u64,
}

impl Default for VaultConfig {
    fn default() -> Self {
        Self {
            min_capital_ratio_bps: 12_000, // 120%
        }
    }
}

/// Per-trigger-type and per-region exposure limits.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskLimits {
    /// Maximum total payout exposure for any single trigger type (WRF base units).
    pub max_exposure_per_trigger_type: Amount,
    /// Maximum total payout exposure for any single geographic region (WRF base units).
    pub max_exposure_per_region: Amount,
}

impl Default for RiskLimits {
    fn default() -> Self {
        Self {
            max_exposure_per_trigger_type: Amount::new(1_000_000_000_000), // 10,000 WRF
            max_exposure_per_region: Amount::new(500_000_000_000),         // 5,000 WRF
        }
    }
}

/// ERC-4626-style insurance vault.
///
/// Tracks:
/// - Underwriter deposits (converted to vault shares).
/// - Total assets (deposits + premiums collected − payouts made).
/// - Active exposure per trigger type and region.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InsuranceVault {
    /// Total WRF assets held in the vault.
    pub total_assets: Amount,
    /// Total shares issued.
    pub total_shares: u128,
    /// Shares owned by each underwriter.
    pub underwriter_shares: HashMap<[u8; 32], VaultShares>,
    /// Total committed payout exposure across all active policies.
    pub total_exposure: Amount,
    /// Exposure per trigger type key.
    pub exposure_by_trigger_type: HashMap<String, Amount>,
    /// Exposure per geographic region.
    pub exposure_by_region: HashMap<String, Amount>,
    /// Vault configuration.
    pub config: VaultConfig,
    /// Risk limits.
    pub risk_limits: RiskLimits,
}

impl InsuranceVault {
    /// Create a new empty vault.
    pub fn new(config: VaultConfig, risk_limits: RiskLimits) -> Self {
        Self {
            total_assets: Amount::ZERO,
            total_shares: 0,
            underwriter_shares: HashMap::new(),
            total_exposure: Amount::ZERO,
            exposure_by_trigger_type: HashMap::new(),
            exposure_by_region: HashMap::new(),
            config,
            risk_limits,
        }
    }

    fn shares_to_assets(&self, shares: u128) -> InsuranceResult<Amount> {
        if self.total_shares == 0 {
            return Ok(Amount::new(shares)); // 1:1 bootstrapping
        }
        self.total_assets
            .0
            .checked_mul(shares)
            .and_then(|v| v.checked_div(self.total_shares))
            .map(Amount::new)
            .ok_or_else(|| InsuranceError::Overflow {
                operation: "shares_to_assets".into(),
            })
    }

    fn assets_to_shares(&self, assets: Amount) -> InsuranceResult<u128> {
        if self.total_assets.is_zero() || self.total_shares == 0 {
            return Ok(assets.0); // 1:1 bootstrapping
        }
        assets
            .0
            .checked_mul(self.total_shares)
            .and_then(|v| v.checked_div(self.total_assets.0))
            .ok_or_else(|| InsuranceError::Overflow {
                operation: "assets_to_shares".into(),
            })
    }

    /// Deposit `amount` of WRF into the vault, minting shares for the underwriter.
    /// Returns the number of shares minted.
    pub fn deposit(&mut self, underwriter: Address, amount: Amount) -> InsuranceResult<VaultShares> {
        if amount.is_zero() {
            return Err(InsuranceError::ZeroDeposit);
        }

        let shares_minted = self.assets_to_shares(amount)?;

        self.total_assets = self
            .total_assets
            .checked_add(amount)
            .ok_or_else(|| InsuranceError::Overflow {
                operation: "vault deposit".into(),
            })?;
        self.total_shares = self
            .total_shares
            .checked_add(shares_minted)
            .ok_or_else(|| InsuranceError::Overflow {
                operation: "vault total_shares".into(),
            })?;

        let entry = self
            .underwriter_shares
            .entry(*underwriter.as_bytes())
            .or_insert(VaultShares::ZERO);
        entry.0 = entry
            .0
            .checked_add(shares_minted)
            .ok_or_else(|| InsuranceError::Overflow {
                operation: "underwriter shares".into(),
            })?;

        Ok(VaultShares::new(shares_minted))
    }

    /// Withdraw by redeeming `shares`. Returns the WRF amount redeemed.
    ///
    /// Fails if the withdrawal would push the capital ratio below the minimum.
    pub fn withdraw(
        &mut self,
        underwriter: Address,
        shares: VaultShares,
    ) -> InsuranceResult<Amount> {
        if shares.is_zero() {
            return Err(InsuranceError::ZeroWithdrawal);
        }

        let held = self
            .underwriter_shares
            .get(underwriter.as_bytes())
            .copied()
            .unwrap_or(VaultShares::ZERO);

        if held < shares {
            return Err(InsuranceError::InsufficientShares {
                underwriter: format!("{}", underwriter),
            });
        }

        let assets_out = self.shares_to_assets(shares.0)?;

        // Check capital ratio after withdrawal.
        let new_total_assets = self
            .total_assets
            .checked_sub(assets_out)
            .ok_or_else(|| InsuranceError::Overflow {
                operation: "vault withdraw".into(),
            })?;

        self.check_capital_ratio_for_assets(new_total_assets)?;

        // Commit changes.
        self.total_assets = new_total_assets;
        self.total_shares = self.total_shares.saturating_sub(shares.0);

        let entry = self
            .underwriter_shares
            .entry(*underwriter.as_bytes())
            .or_insert(VaultShares::ZERO);
        entry.0 = entry.0.saturating_sub(shares.0);

        Ok(assets_out)
    }

    /// Add collected premium to vault assets (called when a policy is issued).
    pub fn collect_premium(&mut self, premium: Amount) -> InsuranceResult<()> {
        self.total_assets = self
            .total_assets
            .checked_add(premium)
            .ok_or_else(|| InsuranceError::Overflow {
                operation: "collect_premium".into(),
            })?;
        Ok(())
    }

    /// Reserve capital for a new policy exposure.
    ///
    /// Checks and applies risk limits: per-trigger-type, per-region, and capital ratio.
    pub fn reserve_exposure(
        &mut self,
        payout: Amount,
        trigger_type_key: &str,
        region_key: &str,
    ) -> InsuranceResult<()> {
        // Check per-trigger-type limit.
        let current_trigger_exp = self
            .exposure_by_trigger_type
            .get(trigger_type_key)
            .copied()
            .unwrap_or(Amount::ZERO);
        let new_trigger_exp = current_trigger_exp
            .checked_add(payout)
            .ok_or_else(|| InsuranceError::Overflow {
                operation: "reserve trigger exposure".into(),
            })?;
        if new_trigger_exp > self.risk_limits.max_exposure_per_trigger_type {
            return Err(InsuranceError::RiskLimitExceeded {
                reason: format!(
                    "trigger type '{}' exposure {}/{} would exceed limit",
                    trigger_type_key,
                    new_trigger_exp.0,
                    self.risk_limits.max_exposure_per_trigger_type.0
                ),
            });
        }

        // Check per-region limit.
        let current_region_exp = self
            .exposure_by_region
            .get(region_key)
            .copied()
            .unwrap_or(Amount::ZERO);
        let new_region_exp = current_region_exp
            .checked_add(payout)
            .ok_or_else(|| InsuranceError::Overflow {
                operation: "reserve region exposure".into(),
            })?;
        if new_region_exp > self.risk_limits.max_exposure_per_region {
            return Err(InsuranceError::RiskLimitExceeded {
                reason: format!(
                    "region '{}' exposure {}/{} would exceed limit",
                    region_key,
                    new_region_exp.0,
                    self.risk_limits.max_exposure_per_region.0
                ),
            });
        }

        // Check capital ratio: free_capital ≥ min_capital_ratio_bps × total_exposure / 10_000
        let new_total_exposure = self
            .total_exposure
            .checked_add(payout)
            .ok_or_else(|| InsuranceError::Overflow {
                operation: "total_exposure".into(),
            })?;
        self.check_capital_ratio_for_exposure(self.total_assets, new_total_exposure)?;

        // Commit.
        self.total_exposure = new_total_exposure;
        *self
            .exposure_by_trigger_type
            .entry(trigger_type_key.to_string())
            .or_insert(Amount::ZERO) = new_trigger_exp;
        *self
            .exposure_by_region
            .entry(region_key.to_string())
            .or_insert(Amount::ZERO) = new_region_exp;

        Ok(())
    }

    /// Release exposure when a policy expires without payout.
    pub fn release_exposure(
        &mut self,
        payout: Amount,
        trigger_type_key: &str,
        region_key: &str,
    ) {
        self.total_exposure = Amount::new(self.total_exposure.0.saturating_sub(payout.0));
        if let Some(v) = self.exposure_by_trigger_type.get_mut(trigger_type_key) {
            v.0 = v.0.saturating_sub(payout.0);
        }
        if let Some(v) = self.exposure_by_region.get_mut(region_key) {
            v.0 = v.0.saturating_sub(payout.0);
        }
    }

    /// Execute a payout, deducting from vault assets.
    pub fn execute_payout(&mut self, payout: Amount) -> InsuranceResult<()> {
        if payout > self.total_assets {
            return Err(InsuranceError::InsufficientCapital {
                required: payout,
                available: self.total_assets,
            });
        }
        self.total_assets = self.total_assets.checked_sub(payout).ok_or_else(|| {
            InsuranceError::Overflow {
                operation: "execute_payout".into(),
            }
        })?;
        Ok(())
    }

    /// Available (free) capital = total_assets − total_exposure.
    pub fn free_capital(&self) -> Amount {
        Amount::new(self.total_assets.0.saturating_sub(self.total_exposure.0))
    }

    /// Actual capital ratio in basis points: free_capital × 10_000 / total_exposure.
    /// Returns `u64::MAX` when there is no exposure (fully de-risked).
    pub fn capital_ratio_bps(&self) -> u64 {
        if self.total_exposure.is_zero() {
            return u64::MAX;
        }
        (self.total_assets.0.saturating_mul(10_000) / self.total_exposure.0) as u64
    }

    // --- Internal helpers ---

    fn check_capital_ratio_for_assets(&self, new_total_assets: Amount) -> InsuranceResult<()> {
        self.check_capital_ratio_for_exposure(new_total_assets, self.total_exposure)
    }

    fn check_capital_ratio_for_exposure(
        &self,
        total_assets: Amount,
        total_exposure: Amount,
    ) -> InsuranceResult<()> {
        if total_exposure.is_zero() {
            return Ok(());
        }
        // required_capital = total_exposure × min_capital_ratio_bps / 10_000
        let required = total_exposure
            .0
            .checked_mul(self.config.min_capital_ratio_bps as u128)
            .and_then(|v| v.checked_div(10_000))
            .ok_or_else(|| InsuranceError::Overflow {
                operation: "capital_ratio_check".into(),
            })?;
        if total_assets.0 < required {
            let actual_bps = (total_assets.0.saturating_mul(10_000)
                / total_exposure.0) as u64;
            return Err(InsuranceError::CapitalRatioBelowMinimum {
                actual_bps,
                min_bps: self.config.min_capital_ratio_bps,
            });
        }
        Ok(())
    }

    /// Shares held by a specific underwriter.
    pub fn shares_of(&self, underwriter: &Address) -> VaultShares {
        self.underwriter_shares
            .get(underwriter.as_bytes())
            .copied()
            .unwrap_or(VaultShares::ZERO)
    }
}

impl Default for InsuranceVault {
    fn default() -> Self {
        Self::new(VaultConfig::default(), RiskLimits::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wrfcoin_defi_common::types::Address;

    fn addr(seed: u8) -> Address {
        let mut bytes = [0u8; 32];
        bytes[31] = seed;
        Address::from_bytes(bytes)
    }

    fn vault() -> InsuranceVault {
        InsuranceVault::new(
            VaultConfig { min_capital_ratio_bps: 10_000 }, // 100%
            RiskLimits {
                max_exposure_per_trigger_type: Amount::new(1_000_000_000),
                max_exposure_per_region: Amount::new(500_000_000),
            },
        )
    }

    #[test]
    fn deposit_mints_shares_one_to_one_initially() {
        let mut v = vault();
        let shares = v.deposit(addr(1), Amount::new(1_000_000)).unwrap();
        assert_eq!(shares, VaultShares::new(1_000_000));
        assert_eq!(v.total_assets, Amount::new(1_000_000));
        assert_eq!(v.total_shares, 1_000_000);
    }

    #[test]
    fn deposit_zero_fails() {
        let mut v = vault();
        assert!(v.deposit(addr(1), Amount::ZERO).is_err());
    }

    #[test]
    fn multiple_underwriters_get_proportional_shares() {
        let mut v = vault();
        let s1 = v.deposit(addr(1), Amount::new(1_000_000)).unwrap();
        let s2 = v.deposit(addr(2), Amount::new(1_000_000)).unwrap();
        assert_eq!(s1.0, s2.0); // equal deposits → equal shares initially
        assert_eq!(v.total_shares, 2_000_000);
        assert_eq!(v.total_assets, Amount::new(2_000_000));
    }

    #[test]
    fn withdraw_returns_correct_assets() {
        let mut v = vault();
        let shares = v.deposit(addr(1), Amount::new(1_000_000)).unwrap();
        let redeemed = v.withdraw(addr(1), shares).unwrap();
        assert_eq!(redeemed, Amount::new(1_000_000));
        assert_eq!(v.total_assets, Amount::ZERO);
        assert_eq!(v.total_shares, 0);
    }

    #[test]
    fn withdraw_insufficient_shares_fails() {
        let mut v = vault();
        v.deposit(addr(1), Amount::new(1_000_000)).unwrap();
        let result = v.withdraw(addr(1), VaultShares::new(2_000_000));
        assert!(result.is_err());
    }

    #[test]
    fn withdraw_with_no_shares_fails() {
        let mut v = vault();
        v.deposit(addr(1), Amount::new(1_000_000)).unwrap();
        let result = v.withdraw(addr(2), VaultShares::new(1)); // addr(2) has no shares
        assert!(result.is_err());
    }

    #[test]
    fn collect_premium_increases_assets() {
        let mut v = vault();
        v.deposit(addr(1), Amount::new(1_000_000)).unwrap();
        v.collect_premium(Amount::new(50_000)).unwrap();
        assert_eq!(v.total_assets, Amount::new(1_050_000));
    }

    #[test]
    fn reserve_exposure_checks_trigger_limit() {
        let mut v = vault();
        v.deposit(addr(1), Amount::new(2_000_000_000)).unwrap();

        // Use distinct regions to avoid hitting the per-region (500M) limit first.
        // Should succeed: 400M < 1B trigger limit
        v.reserve_exposure(Amount::new(400_000_000), "TemperatureAbove", "region-a").unwrap();
        // Should succeed: 400M + 400M = 800M < 1B trigger limit
        v.reserve_exposure(Amount::new(400_000_000), "TemperatureAbove", "region-b").unwrap();
        // Should fail: 800M + 300M = 1.1B > 1B trigger limit
        let result = v.reserve_exposure(Amount::new(300_000_000), "TemperatureAbove", "region-c");
        assert!(result.is_err());
    }

    #[test]
    fn reserve_exposure_checks_region_limit() {
        let mut v = vault();
        v.deposit(addr(1), Amount::new(2_000_000_000)).unwrap();

        // Should succeed: 400M < 500M region limit
        v.reserve_exposure(Amount::new(400_000_000), "TemperatureAbove", "40,-70").unwrap();
        // Should fail: 400M + 200M > 500M region limit
        let result = v.reserve_exposure(Amount::new(200_000_000), "WindSpeedAbove", "40,-70");
        assert!(result.is_err());
    }

    #[test]
    fn capital_ratio_below_minimum_fails() {
        let mut v = vault();
        v.deposit(addr(1), Amount::new(500_000_000)).unwrap();

        // Expose 600M; capital is only 500M → 500/600 = 83% < 100% min
        let result = v.reserve_exposure(Amount::new(600_000_000), "WindSpeedAbove", "region");
        assert!(result.is_err());
    }

    #[test]
    fn execute_payout_deducts_assets() {
        let mut v = vault();
        v.deposit(addr(1), Amount::new(1_000_000)).unwrap();
        v.execute_payout(Amount::new(400_000)).unwrap();
        assert_eq!(v.total_assets, Amount::new(600_000));
    }

    #[test]
    fn execute_payout_fails_if_insufficient_capital() {
        let mut v = vault();
        v.deposit(addr(1), Amount::new(100_000)).unwrap();
        let result = v.execute_payout(Amount::new(200_000));
        assert!(result.is_err());
    }

    #[test]
    fn release_exposure_removes_correctly() {
        let mut v = vault();
        v.deposit(addr(1), Amount::new(2_000_000_000)).unwrap();
        v.reserve_exposure(Amount::new(500_000_000), "TemperatureAbove", "region").unwrap();
        v.release_exposure(Amount::new(500_000_000), "TemperatureAbove", "region");
        assert_eq!(v.total_exposure, Amount::ZERO);
    }

    #[test]
    fn shares_of_returns_zero_for_unknown_underwriter() {
        let v = vault();
        assert_eq!(v.shares_of(&addr(99)), VaultShares::ZERO);
    }

    #[test]
    fn capital_ratio_max_when_no_exposure() {
        let mut v = vault();
        v.deposit(addr(1), Amount::new(1_000_000)).unwrap();
        assert_eq!(v.capital_ratio_bps(), u64::MAX);
    }
}
