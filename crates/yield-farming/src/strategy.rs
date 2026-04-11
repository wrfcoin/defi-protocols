use serde::{Deserialize, Serialize};
use wrfcoin_defi_common::error::{DefiError, DefiResult};
use wrfcoin_defi_common::types::{Amount, Timestamp};

/// Trait for yield optimization strategies that auto-compound or manage rewards.
pub trait YieldStrategy {
    /// Calculate the optimal action given current state.
    /// Returns the amount that should be reinvested.
    fn calculate_reinvestment(
        &self,
        pending_rewards: Amount,
        total_staked: Amount,
        timestamp: Timestamp,
    ) -> DefiResult<Amount>;

    /// Compute the projected APY in basis points given current parameters.
    fn projected_apy_bps(
        &self,
        reward_rate_per_second: Amount,
        total_staked: Amount,
        reward_price_num: u128,
        reward_price_den: u128,
        stake_price_num: u128,
        stake_price_den: u128,
    ) -> DefiResult<u64>;
}

/// A compounding strategy that reinvests all harvested rewards back into the farm.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompoundingStrategy {
    /// Minimum reward amount before a compound is worthwhile (to cover gas costs).
    pub min_compound_threshold: Amount,
    /// Interval in seconds between compound operations.
    pub compound_interval_secs: u64,
    /// Timestamp of the last compound action.
    pub last_compound_time: Timestamp,
    /// Performance fee in basis points taken from rewards before compounding (e.g., 100 = 1%).
    pub performance_fee_bps: u64,
}

/// Deterministic inflation projection for stacked reward scenarios.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct InflationProjection {
    pub additive_total: Amount,
    pub stacked_total: Amount,
    pub capped_total: Amount,
    pub boosted_farm_apr_bps: u64,
    pub boosted_farm_apy_bps: u64,
    pub capped_farm_apr_bps: u64,
    pub capped_farm_apy_bps: u64,
}

impl CompoundingStrategy {
    pub fn new(
        min_compound_threshold: Amount,
        compound_interval_secs: u64,
        performance_fee_bps: u64,
    ) -> Self {
        Self {
            min_compound_threshold,
            compound_interval_secs,
            performance_fee_bps,
            last_compound_time: Timestamp::new(0),
        }
    }

    /// Check whether it is time to compound based on the interval.
    pub fn should_compound(&self, current_time: Timestamp) -> bool {
        current_time.elapsed_since(self.last_compound_time) >= self.compound_interval_secs
    }

    /// Deduct performance fee from a reward amount and return the net amount.
    pub fn deduct_fee(&self, reward: Amount) -> DefiResult<(Amount, Amount)> {
        let fee = reward
            .mul_fraction(self.performance_fee_bps as u128, 10_000)
            .ok_or(DefiError::Overflow {
                operation: "compound fee calculation".into(),
            })?;
        let net = reward.checked_sub(fee).ok_or(DefiError::Overflow {
            operation: "compound fee deduction".into(),
        })?;
        Ok((net, fee))
    }

    fn approximate_apy_from_apr_bps(&self, apr_bps: u64) -> u64 {
        let boost = (apr_bps as u128)
            .checked_mul(apr_bps as u128)
            .and_then(|value| value.checked_div(20_000))
            .unwrap_or(0);
        (apr_bps as u128)
            .saturating_add(boost)
            .min(u64::MAX as u128) as u64
    }

    pub fn project_inflation_path(
        &self,
        principal: Amount,
        l1_yield_bps: u64,
        farm_base_apr_bps: u64,
        reward_multiplier_bps: u64,
        address_multiplier_limit_bps: u64,
    ) -> DefiResult<InflationProjection> {
        let boosted_farm_apr_bps = farm_base_apr_bps
            .checked_mul(reward_multiplier_bps)
            .and_then(|value| value.checked_div(10_000))
            .ok_or(DefiError::Overflow {
                operation: "projection boosted farm apr".into(),
            })?;
        let capped_farm_apr_bps = farm_base_apr_bps
            .checked_mul(reward_multiplier_bps.min(address_multiplier_limit_bps))
            .and_then(|value| value.checked_div(10_000))
            .ok_or(DefiError::Overflow {
                operation: "projection capped farm apr".into(),
            })?;
        let boosted_farm_apy_bps = self.approximate_apy_from_apr_bps(boosted_farm_apr_bps);
        let capped_farm_apy_bps = self.approximate_apy_from_apr_bps(capped_farm_apr_bps);

        let additive_total = principal
            .checked_add(principal.mul_fraction(l1_yield_bps as u128, 10_000).ok_or(
                DefiError::Overflow {
                    operation: "projection additive l1 gain".into(),
                },
            )?)
            .and_then(|value| {
                value.checked_add(
                    principal
                        .mul_fraction(boosted_farm_apr_bps as u128, 10_000)
                        .expect("boosted_farm_apr_bps denominator is non-zero"),
                )
            })
            .ok_or(DefiError::Overflow {
                operation: "projection additive total".into(),
            })?;
        let stacked_total = principal
            .mul_fraction(10_000 + l1_yield_bps as u128, 10_000)
            .and_then(|value| value.mul_fraction(10_000 + boosted_farm_apy_bps as u128, 10_000))
            .ok_or(DefiError::Overflow {
                operation: "projection stacked total".into(),
            })?;
        let capped_total = principal
            .mul_fraction(10_000 + l1_yield_bps as u128, 10_000)
            .and_then(|value| value.mul_fraction(10_000 + capped_farm_apy_bps as u128, 10_000))
            .ok_or(DefiError::Overflow {
                operation: "projection capped total".into(),
            })?;

        Ok(InflationProjection {
            additive_total,
            stacked_total,
            capped_total,
            boosted_farm_apr_bps,
            boosted_farm_apy_bps,
            capped_farm_apr_bps,
            capped_farm_apy_bps,
        })
    }
}

impl YieldStrategy for CompoundingStrategy {
    fn calculate_reinvestment(
        &self,
        pending_rewards: Amount,
        _total_staked: Amount,
        timestamp: Timestamp,
    ) -> DefiResult<Amount> {
        // Only compound if enough time has passed and rewards exceed threshold.
        if !self.should_compound(timestamp) {
            return Ok(Amount::ZERO);
        }
        if pending_rewards < self.min_compound_threshold {
            return Ok(Amount::ZERO);
        }

        // Reinvest everything after the performance fee.
        let (net, _fee) = self.deduct_fee(pending_rewards)?;
        Ok(net)
    }

    fn projected_apy_bps(
        &self,
        reward_rate_per_second: Amount,
        total_staked: Amount,
        reward_price_num: u128,
        reward_price_den: u128,
        stake_price_num: u128,
        stake_price_den: u128,
    ) -> DefiResult<u64> {
        if total_staked.is_zero() || stake_price_num == 0 || reward_price_den == 0 {
            return Ok(0);
        }

        let seconds_per_year: u128 = 365 * 24 * 3600;

        // Annual reward value = reward_rate * seconds_per_year * reward_price
        let annual_reward_tokens = reward_rate_per_second
            .0
            .checked_mul(seconds_per_year)
            .ok_or(DefiError::Overflow {
                operation: "apy annual_reward".into(),
            })?;

        let annual_reward_value = annual_reward_tokens
            .checked_mul(reward_price_num)
            .and_then(|v| v.checked_div(reward_price_den))
            .ok_or(DefiError::Overflow {
                operation: "apy reward_value".into(),
            })?;

        // Staked value = total_staked * stake_price
        let staked_value = total_staked
            .0
            .checked_mul(stake_price_num)
            .and_then(|v| v.checked_div(stake_price_den))
            .ok_or(DefiError::Overflow {
                operation: "apy staked_value".into(),
            })?;

        if staked_value == 0 {
            return Ok(0);
        }

        // APR (bps) = annual_reward_value * 10_000 / staked_value
        let apr_bps = annual_reward_value
            .checked_mul(10_000)
            .and_then(|v| v.checked_div(staked_value))
            .ok_or(DefiError::Overflow {
                operation: "apy apr".into(),
            })?;

        Ok(self.approximate_apy_from_apr_bps(apr_bps.min(u64::MAX as u128) as u64))
    }
}

impl Default for CompoundingStrategy {
    fn default() -> Self {
        Self::new(
            Amount::new(1_000_000), // Min 1M smallest-unit tokens to compound
            3600,                   // Compound every hour
            100,                    // 1% performance fee
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Quantified projection table used to document the stacked-inflation envelope.
    ///
    /// | Scenario | L1 Yield | Farm APR | Raw Boost | Address Cap | Additive Total | Stacked Total | Capped Total |
    /// | --- | --- | --- | --- | --- | ---: | ---: | ---: |
    /// | Launch worst-case | 8.00% | 12.00% | 1.50x | 1.25x | 126000 | 129189 | 125409 |
    /// | Moderate stacked boost | 6.00% | 10.00% | 1.35x | 1.20x | 119500 | 121274 | 119483 |
    /// | Conservative utility tilt | 4.00% | 8.00% | 1.20x | 1.15x | 113600 | 114462 | 114004 |

    #[test]
    fn test_should_compound_timing() {
        let strategy = CompoundingStrategy::new(Amount::ZERO, 3600, 100);
        // At t=0, last_compound = 0, so elapsed >= 3600 is false
        assert!(!strategy.should_compound(Timestamp::new(100)));
        assert!(strategy.should_compound(Timestamp::new(3600)));
        assert!(strategy.should_compound(Timestamp::new(7200)));
    }

    #[test]
    fn test_deduct_fee() {
        let strategy = CompoundingStrategy::new(Amount::ZERO, 3600, 100); // 1%
        let (net, fee) = strategy.deduct_fee(Amount::new(10_000)).unwrap();
        assert_eq!(fee, Amount::new(100)); // 1%
        assert_eq!(net, Amount::new(9_900));
    }

    #[test]
    fn test_calculate_reinvestment_below_threshold() {
        let strategy = CompoundingStrategy::new(Amount::new(1000), 3600, 100);
        let result = strategy
            .calculate_reinvestment(Amount::new(500), Amount::new(10_000), Timestamp::new(3600))
            .unwrap();
        assert_eq!(result, Amount::ZERO); // Below threshold
    }

    #[test]
    fn test_calculate_reinvestment_not_time_yet() {
        let strategy = CompoundingStrategy::new(Amount::new(100), 3600, 100);
        let result = strategy
            .calculate_reinvestment(Amount::new(1000), Amount::new(10_000), Timestamp::new(100))
            .unwrap();
        assert_eq!(result, Amount::ZERO); // Not enough time elapsed
    }

    #[test]
    fn test_calculate_reinvestment_success() {
        let strategy = CompoundingStrategy::new(Amount::new(100), 3600, 100);
        let result = strategy
            .calculate_reinvestment(
                Amount::new(10_000),
                Amount::new(100_000),
                Timestamp::new(3600),
            )
            .unwrap();
        // net = 10_000 - 1% fee = 9_900
        assert_eq!(result, Amount::new(9_900));
    }

    #[test]
    fn test_projected_apy_basic() {
        let strategy = CompoundingStrategy::default();
        // 100 tokens/sec, 1M staked, price 1:1 for both
        let apy = strategy
            .projected_apy_bps(Amount::new(100), Amount::new(1_000_000), 1, 1, 1, 1)
            .unwrap();
        // APR = 100 * 31_536_000 / 1_000_000 * 10_000 = 31_536_000_000 ... very high
        assert!(apy > 0);
    }

    #[test]
    fn test_projected_apy_zero_staked() {
        let strategy = CompoundingStrategy::default();
        let apy = strategy
            .projected_apy_bps(Amount::new(100), Amount::ZERO, 1, 1, 1, 1)
            .unwrap();
        assert_eq!(apy, 0);
    }

    #[test]
    fn test_stacked_emission_path_amplification() {
        let strategy = CompoundingStrategy::default();

        let projection = strategy
            .project_inflation_path(Amount::new(100_000), 800, 1_200, 15_000, 12_500)
            .expect("projection should succeed");

        assert_eq!(projection.boosted_farm_apr_bps, 1_800);
        assert_eq!(projection.boosted_farm_apy_bps, 1_962);
        assert_eq!(projection.capped_farm_apr_bps, 1_500);
        assert_eq!(projection.capped_farm_apy_bps, 1_612);
        assert_eq!(projection.additive_total, Amount::new(126_000));
        assert_eq!(projection.stacked_total, Amount::new(129_189));
        assert_eq!(projection.capped_total, Amount::new(125_409));
        assert_eq!(
            projection
                .stacked_total
                .checked_sub(projection.additive_total)
                .expect("stacked total should exceed additive total"),
            Amount::new(3_189)
        );
    }

    #[test]
    fn test_quantified_inflation_projection_table() {
        let strategy = CompoundingStrategy::default();
        let principal = Amount::new(100_000);
        let scenarios = [
            (
                "Launch worst-case",
                strategy
                    .project_inflation_path(principal, 800, 1_200, 15_000, 12_500)
                    .unwrap(),
                Amount::new(126_000),
                Amount::new(129_189),
                Amount::new(125_409),
            ),
            (
                "Moderate stacked boost",
                strategy
                    .project_inflation_path(principal, 600, 1_000, 13_500, 12_000)
                    .unwrap(),
                Amount::new(119_500),
                Amount::new(121_274),
                Amount::new(119_483),
            ),
            (
                "Conservative utility tilt",
                strategy
                    .project_inflation_path(principal, 400, 800, 12_000, 11_500)
                    .unwrap(),
                Amount::new(113_600),
                Amount::new(114_462),
                Amount::new(114_004),
            ),
        ];

        for (label, projection, additive_total, stacked_total, capped_total) in scenarios {
            assert_eq!(projection.additive_total, additive_total, "{label}");
            assert_eq!(projection.stacked_total, stacked_total, "{label}");
            assert_eq!(projection.capped_total, capped_total, "{label}");
            assert!(
                projection.capped_total <= projection.stacked_total,
                "{label}"
            );
        }
    }
}
