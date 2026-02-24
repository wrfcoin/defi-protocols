use serde::{Deserialize, Serialize};
use wrfcoin_defi_common::error::{DefiError, DefiResult};
use wrfcoin_defi_common::types::{Amount, Timestamp};

/// Tracks a linear vesting schedule for reward tokens.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VestingSchedule {
    /// Total amount of tokens to vest.
    pub total: Amount,
    /// Amount already claimed by the beneficiary.
    pub claimed: Amount,
    /// Timestamp when vesting begins.
    pub start_time: Timestamp,
    /// Duration of the vesting period in seconds.
    pub duration_secs: u64,
}

impl VestingSchedule {
    pub fn new(total: Amount, start_time: Timestamp, duration_secs: u64) -> Self {
        Self {
            total,
            claimed: Amount::ZERO,
            start_time,
            duration_secs,
        }
    }

    /// Calculate the total vested amount at a given timestamp (linear vesting).
    pub fn vested_at(&self, timestamp: Timestamp) -> Amount {
        if timestamp <= self.start_time {
            return Amount::ZERO;
        }

        let elapsed = timestamp.elapsed_since(self.start_time);

        if elapsed >= self.duration_secs || self.duration_secs == 0 {
            // Fully vested.
            return self.total;
        }

        // Linear: total * elapsed / duration
        let vested = (self.total.0 as u128)
            .checked_mul(elapsed as u128)
            .and_then(|v| v.checked_div(self.duration_secs as u128))
            .unwrap_or(0);

        Amount(vested)
    }

    /// Calculate the claimable (vested but not yet claimed) amount.
    pub fn claimable_at(&self, timestamp: Timestamp) -> Amount {
        let vested = self.vested_at(timestamp);
        Amount(vested.0.saturating_sub(self.claimed.0))
    }

    /// Record a claim, returning the claimed amount.
    pub fn claim(&mut self, timestamp: Timestamp) -> DefiResult<Amount> {
        let claimable = self.claimable_at(timestamp);
        if claimable.is_zero() {
            return Err(DefiError::InvalidAmount {
                reason: "nothing to claim at this time".into(),
            });
        }
        self.claimed = self.claimed.checked_add(claimable).ok_or(DefiError::Overflow {
            operation: "vesting claim".into(),
        })?;
        Ok(claimable)
    }

    /// Check if the schedule is fully vested and claimed.
    pub fn is_complete(&self) -> bool {
        self.claimed >= self.total
    }
}

/// Calculates pending rewards using the standard MasterChef-style accumulator pattern.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RewardCalculator {
    /// Reward tokens emitted per second.
    pub reward_per_second: Amount,
}

impl RewardCalculator {
    pub fn new(reward_per_second: Amount) -> Self {
        Self { reward_per_second }
    }

    /// Calculate the pending reward for a user based on their share of the pool.
    ///
    /// # Arguments
    /// * `user_staked` - Amount the user has staked
    /// * `total_staked` - Total amount staked in the pool
    /// * `acc_reward_per_share` - Current accumulated reward per share (scaled)
    /// * `user_reward_debt` - User's reward debt
    /// * `precision` - Precision multiplier used for acc_reward_per_share
    pub fn calculate_pending_reward(
        &self,
        user_staked: Amount,
        acc_reward_per_share: u128,
        user_reward_debt: u128,
        precision: u128,
    ) -> Amount {
        if user_staked.is_zero() || precision == 0 {
            return Amount::ZERO;
        }

        let accumulated = user_staked
            .0
            .saturating_mul(acc_reward_per_share)
            / precision;

        Amount(accumulated.saturating_sub(user_reward_debt))
    }

    /// Calculate the vested amount from a vesting schedule at a given time.
    pub fn calculate_vested_amount(
        &self,
        schedule: &VestingSchedule,
        timestamp: Timestamp,
    ) -> Amount {
        schedule.vested_at(timestamp)
    }
}

impl Default for RewardCalculator {
    fn default() -> Self {
        Self::new(Amount::ZERO)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vesting_before_start() {
        let sched = VestingSchedule::new(Amount::new(1000), Timestamp::new(100), 1000);
        assert_eq!(sched.vested_at(Timestamp::new(50)), Amount::ZERO);
    }

    #[test]
    fn test_vesting_at_start() {
        let sched = VestingSchedule::new(Amount::new(1000), Timestamp::new(100), 1000);
        assert_eq!(sched.vested_at(Timestamp::new(100)), Amount::ZERO);
    }

    #[test]
    fn test_vesting_halfway() {
        let sched = VestingSchedule::new(Amount::new(1000), Timestamp::new(100), 1000);
        assert_eq!(sched.vested_at(Timestamp::new(600)), Amount::new(500));
    }

    #[test]
    fn test_vesting_complete() {
        let sched = VestingSchedule::new(Amount::new(1000), Timestamp::new(100), 1000);
        assert_eq!(sched.vested_at(Timestamp::new(1200)), Amount::new(1000));
    }

    #[test]
    fn test_vesting_zero_duration() {
        let sched = VestingSchedule::new(Amount::new(1000), Timestamp::new(100), 0);
        // Zero duration means instant vesting
        assert_eq!(sched.vested_at(Timestamp::new(101)), Amount::new(1000));
    }

    #[test]
    fn test_claimable() {
        let sched = VestingSchedule::new(Amount::new(1000), Timestamp::new(0), 1000);
        assert_eq!(sched.claimable_at(Timestamp::new(500)), Amount::new(500));
    }

    #[test]
    fn test_claim() {
        let mut sched = VestingSchedule::new(Amount::new(1000), Timestamp::new(0), 1000);
        let claimed = sched.claim(Timestamp::new(500)).unwrap();
        assert_eq!(claimed, Amount::new(500));
        assert_eq!(sched.claimed, Amount::new(500));
        // Can't claim again at same time
        assert!(sched.claim(Timestamp::new(500)).is_err());
    }

    #[test]
    fn test_claim_progressive() {
        let mut sched = VestingSchedule::new(Amount::new(1000), Timestamp::new(0), 1000);
        sched.claim(Timestamp::new(500)).unwrap(); // 500
        let second = sched.claim(Timestamp::new(800)).unwrap(); // 300 more
        assert_eq!(second, Amount::new(300));
        assert_eq!(sched.claimed, Amount::new(800));
    }

    #[test]
    fn test_is_complete() {
        let mut sched = VestingSchedule::new(Amount::new(1000), Timestamp::new(0), 1000);
        assert!(!sched.is_complete());
        sched.claim(Timestamp::new(1100)).unwrap();
        assert!(sched.is_complete());
    }

    #[test]
    fn test_reward_calculator_pending() {
        let calc = RewardCalculator::new(Amount::new(100));
        let precision = 1_000_000_000_000u128;
        // User has 500 staked, acc_reward = 2 * precision, debt = 500 * precision
        let acc = 2 * precision;
        let debt = 500 * precision / precision; // simplified
        let pending = calc.calculate_pending_reward(Amount::new(500), acc, debt, precision);
        // accumulated = 500 * 2 * precision / precision = 1000
        // pending = 1000 - debt
        assert_eq!(pending, Amount::new(1000 - debt));
    }
}
