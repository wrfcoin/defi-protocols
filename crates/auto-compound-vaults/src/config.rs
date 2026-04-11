use serde::{Deserialize, Serialize};
use wrfcoin_defi_common::error::{DefiError, DefiResult};
use wrfcoin_defi_common::types::Amount;
use wrfcoin_yield_farming::strategy::CompoundingStrategy;

/// Per-vault configuration for an auto-compounding vault.
///
/// All three fields map directly onto [`CompoundingStrategy`] parameters so
/// that the vault manager can create/update strategies from user-supplied
/// configs without knowing the internals of the yield-farming crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct VaultConfig {
    /// Minimum accumulated reward amount (in base units) before a compound
    /// cycle is worth triggering.  Prevents dust-compounds that cost more in
    /// overhead than they earn.
    pub min_reward_threshold: Amount,

    /// Minimum wall-clock seconds that must elapse between compound cycles.
    /// This is an independent guard from `min_reward_threshold`; both must be
    /// satisfied before a compound fires.
    pub compound_interval_secs: u64,

    /// Performance fee in basis points (1 bps = 0.01 %) deducted from
    /// harvested rewards before reinvestment.  Max 2 000 bps (20 %).
    pub performance_fee_bps: u64,
}

impl VaultConfig {
    /// Maximum allowed performance fee: 20 % (2 000 bps).
    pub const MAX_PERFORMANCE_FEE_BPS: u64 = 2_000;

    /// Build and validate a `VaultConfig`.
    pub fn new(
        min_reward_threshold: Amount,
        compound_interval_secs: u64,
        performance_fee_bps: u64,
    ) -> DefiResult<Self> {
        if compound_interval_secs == 0 {
            return Err(DefiError::InvalidAmount {
                reason: "compound_interval_secs must be > 0".into(),
            });
        }
        if performance_fee_bps > Self::MAX_PERFORMANCE_FEE_BPS {
            return Err(DefiError::InvalidAmount {
                reason: format!(
                    "performance_fee_bps {} exceeds maximum {} bps",
                    performance_fee_bps,
                    Self::MAX_PERFORMANCE_FEE_BPS,
                ),
            });
        }
        Ok(Self {
            min_reward_threshold,
            compound_interval_secs,
            performance_fee_bps,
        })
    }

    /// Conservative defaults suitable for production launch.
    ///
    /// - `min_reward_threshold`: 1 WRF (10^8 base units)
    /// - `compound_interval_secs`: 3 600 (1 hour)
    /// - `performance_fee_bps`: 100 (1 %)
    pub fn default_production() -> Self {
        Self {
            min_reward_threshold: Amount::new(100_000_000),
            compound_interval_secs: 3_600,
            performance_fee_bps: 100,
        }
    }

    /// Convert this config into a [`CompoundingStrategy`] for the yield-farming
    /// crate.
    pub fn into_strategy(self) -> CompoundingStrategy {
        CompoundingStrategy::new(
            self.min_reward_threshold,
            self.compound_interval_secs,
            self.performance_fee_bps,
        )
    }
}

impl Default for VaultConfig {
    fn default() -> Self {
        Self::default_production()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_rejects_zero_interval() {
        let err = VaultConfig::new(Amount::new(100), 0, 50).unwrap_err();
        assert!(matches!(err, DefiError::InvalidAmount { .. }));
    }

    #[test]
    fn config_rejects_fee_above_cap() {
        let err = VaultConfig::new(Amount::new(100), 3_600, 2_001).unwrap_err();
        assert!(matches!(err, DefiError::InvalidAmount { .. }));
    }

    #[test]
    fn config_accepts_max_fee() {
        let cfg = VaultConfig::new(Amount::new(100), 3_600, 2_000).unwrap();
        assert_eq!(cfg.performance_fee_bps, 2_000);
    }

    #[test]
    fn config_into_strategy_maps_fields() {
        let cfg = VaultConfig::new(Amount::new(500), 1_800, 150).unwrap();
        let strategy = cfg.into_strategy();
        assert_eq!(strategy.min_compound_threshold, Amount::new(500));
        assert_eq!(strategy.compound_interval_secs, 1_800);
        assert_eq!(strategy.performance_fee_bps, 150);
    }

    #[test]
    fn default_production_config_is_valid() {
        let cfg = VaultConfig::default_production();
        assert_eq!(cfg.compound_interval_secs, 3_600);
        assert_eq!(cfg.performance_fee_bps, 100);
        // Validate passes
        let rebuilt = VaultConfig::new(
            cfg.min_reward_threshold,
            cfg.compound_interval_secs,
            cfg.performance_fee_bps,
        )
        .unwrap();
        assert_eq!(rebuilt, cfg);
    }
}
