use serde::{Deserialize, Serialize};
use wrfcoin_defi_common::error::{DefiError, DefiResult};
use wrfcoin_defi_common::types::{Amount, UtilitySignalSnapshot};

/// Configuration for utility-linked reward weighting.
///
/// Multipliers are expressed in basis points and are clamped to:
/// - `min_multiplier_bps` <= multiplier <= `max_multiplier_bps`
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct UtilityRewardHookConfig {
    /// Minimum multiplier applied to base rewards (e.g. 6000 = 0.6x).
    pub min_multiplier_bps: u16,
    /// Neutral multiplier (e.g. 10000 = 1.0x).
    pub neutral_multiplier_bps: u16,
    /// Maximum multiplier applied to base rewards (e.g. 15000 = 1.5x).
    pub max_multiplier_bps: u16,
    /// Maximum multiplier any single address can realize over its baseline rewards.
    pub per_address_multiplier_limit_bps: u16,
    /// Weight assigned to composite usefulness score.
    pub utility_score_weight_bps: u16,
    /// Weight assigned to verified-event ratio score.
    pub verified_ratio_weight_bps: u16,
    /// Threshold used for classifying rewards as utility-linked.
    pub utility_linked_threshold_bps: u16,
}

impl Default for UtilityRewardHookConfig {
    fn default() -> Self {
        Self {
            min_multiplier_bps: 6_000,
            neutral_multiplier_bps: 10_000,
            max_multiplier_bps: 15_000,
            per_address_multiplier_limit_bps: 12_500,
            utility_score_weight_bps: 7_000,
            verified_ratio_weight_bps: 3_000,
            utility_linked_threshold_bps: 6_000,
        }
    }
}

impl UtilityRewardHookConfig {
    pub fn validate(&self) -> DefiResult<()> {
        if self.min_multiplier_bps == 0 || self.max_multiplier_bps < self.neutral_multiplier_bps {
            return Err(DefiError::InvalidAmount {
                reason: "invalid utility multiplier bounds".into(),
            });
        }
        if self.min_multiplier_bps > self.neutral_multiplier_bps {
            return Err(DefiError::InvalidAmount {
                reason: "min_multiplier_bps must be <= neutral_multiplier_bps".into(),
            });
        }
        if self.per_address_multiplier_limit_bps < self.neutral_multiplier_bps
            || self.per_address_multiplier_limit_bps > self.max_multiplier_bps
        {
            return Err(DefiError::InvalidAmount {
                reason: "per_address_multiplier_limit_bps must be between neutral and max".into(),
            });
        }
        let total_weight =
            self.utility_score_weight_bps as u32 + self.verified_ratio_weight_bps as u32;
        if total_weight == 0 || total_weight != UtilitySignalSnapshot::BPS_SCALE as u32 {
            return Err(DefiError::InvalidAmount {
                reason: "utility hook weights must sum to 10_000 bps".into(),
            });
        }
        if self.utility_linked_threshold_bps > UtilitySignalSnapshot::BPS_SCALE {
            return Err(DefiError::InvalidAmount {
                reason: "utility_linked_threshold_bps must be <= 10_000".into(),
            });
        }
        Ok(())
    }
}

/// Utility-weighted reward hook model.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
pub struct UtilityRewardHook {
    pub config: UtilityRewardHookConfig,
}

/// Backtest input point for utility-linked reward simulation.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct UtilityBacktestPoint {
    /// Baseline reward before utility weighting.
    pub base_reward: Amount,
    /// Utility signal snapshot used to derive reward multiplier.
    pub signal: UtilitySignalSnapshot,
}

/// Telemetry summary for utility-linked reward backtests.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct UtilityBacktestTelemetry {
    /// Sum of unweighted baseline rewards.
    pub baseline_rewards: Amount,
    /// Sum of weighted rewards after utility hooks.
    pub weighted_rewards: Amount,
    /// Weighted rewards classified as utility-linked.
    pub utility_linked_rewards: Amount,
    /// Weighted rewards classified as primarily financial activity.
    pub financial_activity_rewards: Amount,
    /// Utility-linked reward ratio in basis points.
    pub utility_reward_ratio_bps: u16,
}

impl Default for UtilityBacktestTelemetry {
    fn default() -> Self {
        Self {
            baseline_rewards: Amount::ZERO,
            weighted_rewards: Amount::ZERO,
            utility_linked_rewards: Amount::ZERO,
            financial_activity_rewards: Amount::ZERO,
            utility_reward_ratio_bps: 0,
        }
    }
}

impl UtilityRewardHook {
    /// Build a hook with explicit config validation.
    pub fn with_config(config: UtilityRewardHookConfig) -> DefiResult<Self> {
        config.validate()?;
        Ok(Self { config })
    }

    /// Aggregate score in basis points combining usefulness and verified ratio.
    pub fn aggregate_score_bps(&self, signal: &UtilitySignalSnapshot) -> u16 {
        let usefulness = signal.composite_usefulness_bps() as u32;
        let verified_ratio = signal.verified_event_ratio_bps() as u32;
        let weighted = usefulness * self.config.utility_score_weight_bps as u32
            + verified_ratio * self.config.verified_ratio_weight_bps as u32;
        (weighted / UtilitySignalSnapshot::BPS_SCALE as u32) as u16
    }

    /// Compute reward multiplier in basis points from a utility signal.
    ///
    /// Score below neutral reduces rewards toward `min_multiplier_bps`.
    /// Score above neutral increases rewards toward `max_multiplier_bps`.
    pub fn multiplier_bps(&self, signal: &UtilitySignalSnapshot) -> u16 {
        let score = self.aggregate_score_bps(signal) as u32;
        let neutral = UtilitySignalSnapshot::NEUTRAL_BPS as u32;
        let min = self.config.min_multiplier_bps as u32;
        let mid = self.config.neutral_multiplier_bps as u32;
        let max = self.config.max_multiplier_bps as u32;

        if score >= neutral {
            let up_span = max.saturating_sub(mid);
            let score_above = score.saturating_sub(neutral);
            let adj = (up_span.saturating_mul(score_above)) / neutral.max(1);
            (mid.saturating_add(adj)).min(max) as u16
        } else {
            let down_span = mid.saturating_sub(min);
            let score_below = neutral.saturating_sub(score);
            let adj = (down_span.saturating_mul(score_below)) / neutral.max(1);
            mid.saturating_sub(adj).max(min) as u16
        }
    }

    /// Apply utility-linked weighting to a baseline reward.
    pub fn apply(&self, base_reward: Amount, signal: &UtilitySignalSnapshot) -> DefiResult<Amount> {
        let multiplier = self.multiplier_bps(signal) as u128;
        base_reward
            .mul_fraction(multiplier, UtilitySignalSnapshot::BPS_SCALE as u128)
            .ok_or(DefiError::Overflow {
                operation: "utility reward hook apply".into(),
            })
    }

    /// Clamp a candidate multiplier to the configured per-address limit.
    pub fn cap_multiplier_bps(&self, multiplier_bps: u16, address_limit_bps: u16) -> u16 {
        multiplier_bps.min(address_limit_bps)
    }

    /// Apply an address-level multiplier cap to an already weighted reward.
    pub fn cap_reward(
        &self,
        base_reward: Amount,
        weighted_reward: Amount,
        address_limit_bps: u16,
    ) -> DefiResult<Amount> {
        let capped_multiplier = self.cap_multiplier_bps(
            self.config.max_multiplier_bps,
            address_limit_bps.max(self.config.neutral_multiplier_bps),
        ) as u128;
        let capped_reward = base_reward
            .mul_fraction(capped_multiplier, UtilitySignalSnapshot::BPS_SCALE as u128)
            .ok_or(DefiError::Overflow {
                operation: "utility reward hook cap".into(),
            })?;
        Ok(weighted_reward.min(capped_reward))
    }

    /// Whether a signal qualifies rewards as utility-linked.
    pub fn is_utility_linked(&self, signal: &UtilitySignalSnapshot) -> bool {
        self.aggregate_score_bps(signal) >= self.config.utility_linked_threshold_bps
    }

    /// Run deterministic backtest telemetry over a reward series.
    pub fn run_backtest(
        &self,
        points: &[UtilityBacktestPoint],
    ) -> DefiResult<UtilityBacktestTelemetry> {
        let mut telemetry = UtilityBacktestTelemetry::default();
        for point in points {
            let weighted = self.apply(point.base_reward, &point.signal)?;
            telemetry.baseline_rewards = telemetry
                .baseline_rewards
                .checked_add(point.base_reward)
                .ok_or(DefiError::Overflow {
                    operation: "utility backtest baseline sum".into(),
                })?;
            telemetry.weighted_rewards =
                telemetry
                    .weighted_rewards
                    .checked_add(weighted)
                    .ok_or(DefiError::Overflow {
                        operation: "utility backtest weighted sum".into(),
                    })?;

            if self.is_utility_linked(&point.signal) {
                telemetry.utility_linked_rewards = telemetry
                    .utility_linked_rewards
                    .checked_add(weighted)
                    .ok_or(DefiError::Overflow {
                        operation: "utility backtest linked sum".into(),
                    })?;
            } else {
                telemetry.financial_activity_rewards = telemetry
                    .financial_activity_rewards
                    .checked_add(weighted)
                    .ok_or(DefiError::Overflow {
                        operation: "utility backtest financial sum".into(),
                    })?;
            }
        }

        if telemetry.weighted_rewards.is_zero() {
            telemetry.utility_reward_ratio_bps = 0;
        } else {
            let ratio = telemetry
                .utility_linked_rewards
                .0
                .saturating_mul(UtilitySignalSnapshot::BPS_SCALE as u128)
                .saturating_div(telemetry.weighted_rewards.0);
            telemetry.utility_reward_ratio_bps =
                ratio.min(UtilitySignalSnapshot::BPS_SCALE as u128) as u16;
        }
        Ok(telemetry)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signal(score: u16, verified_ratio: u16) -> UtilitySignalSnapshot {
        UtilitySignalSnapshot {
            forecast_usefulness_bps: score,
            report_usefulness_bps: score,
            verification_confidence_bps: UtilitySignalSnapshot::BPS_SCALE,
            verified_events: verified_ratio as u64,
            unverified_events: (UtilitySignalSnapshot::BPS_SCALE - verified_ratio) as u64,
        }
    }

    #[test]
    fn utility_multiplier_increases_with_stronger_verified_signal() {
        let hook = UtilityRewardHook::default();
        let low = signal(3_000, 2_000);
        let high = signal(9_000, 9_000);

        assert!(hook.multiplier_bps(&low) < hook.config.neutral_multiplier_bps);
        assert!(hook.multiplier_bps(&high) > hook.config.neutral_multiplier_bps);
    }

    #[test]
    fn backtest_tracks_utility_reward_ratio() {
        let hook = UtilityRewardHook::default();
        let points = vec![
            UtilityBacktestPoint {
                base_reward: Amount::new(1_000),
                signal: signal(8_500, 9_000),
            },
            UtilityBacktestPoint {
                base_reward: Amount::new(1_000),
                signal: signal(7_500, 8_000),
            },
            UtilityBacktestPoint {
                base_reward: Amount::new(1_000),
                signal: signal(2_500, 2_000),
            },
        ];

        let telemetry = hook.run_backtest(&points).unwrap();
        assert!(telemetry.weighted_rewards > Amount::ZERO);
        assert!(telemetry.utility_linked_rewards > telemetry.financial_activity_rewards);
        assert!(telemetry.utility_reward_ratio_bps > 5_000);
    }

    #[test]
    fn invalid_config_rejected() {
        let bad = UtilityRewardHookConfig {
            utility_score_weight_bps: 6_000,
            verified_ratio_weight_bps: 1_000,
            ..Default::default()
        };
        assert!(UtilityRewardHook::with_config(bad).is_err());
    }

    #[test]
    fn per_address_cap_clamps_boosted_reward() {
        let hook = UtilityRewardHook::default();
        let base_reward = Amount::new(1_000);
        let weighted_reward = hook.apply(base_reward, &signal(9_000, 9_000)).unwrap();

        let capped = hook
            .cap_reward(base_reward, weighted_reward, 11_000)
            .unwrap();

        assert_eq!(capped, Amount::new(1_100));
        assert!(capped < weighted_reward);
    }
}
