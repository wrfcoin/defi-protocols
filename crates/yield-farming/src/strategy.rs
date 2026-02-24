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

        // For a simple compounding estimate with n compounds per year:
        // APY ~= APR * (1 + small_boost) — use a simple approximation for the scaffold.
        // Compound boost: APY_bps = APR_bps + APR_bps^2 / (2 * 10_000)
        let boost = apr_bps
            .checked_mul(apr_bps)
            .and_then(|v| v.checked_div(20_000))
            .unwrap_or(0);

        let apy_bps = apr_bps.saturating_add(boost);

        // Clamp to u64.
        Ok(apy_bps.min(u64::MAX as u128) as u64)
    }
}

impl Default for CompoundingStrategy {
    fn default() -> Self {
        Self::new(
            Amount::new(1_000_000),   // Min 1M smallest-unit tokens to compound
            3600,                      // Compound every hour
            100,                       // 1% performance fee
        )
    }
}
