use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use wrfcoin_defi_common::error::{DefiError, DefiResult};
use wrfcoin_defi_common::types::{Address, Amount, AssetId, PoolId, Timestamp};

/// Precision multiplier for accumulated reward calculations to avoid rounding losses.
const ACC_REWARD_PRECISION: u128 = 1_000_000_000_000;

/// Represents a single farming pool where users can stake LP tokens for rewards.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FarmPool {
    /// Unique identifier for this farm pool.
    pub pool_id: PoolId,
    /// The LP token that can be staked in this pool.
    pub lp_token: AssetId,
    /// Allocation points for this pool (determines share of total rewards).
    pub alloc_point: u64,
    /// Accumulated reward per share, scaled by ACC_REWARD_PRECISION.
    pub acc_reward_per_share: u128,
    /// Total amount of LP tokens staked in this pool.
    pub total_staked: Amount,
    /// Timestamp of the last reward distribution update.
    pub last_reward_time: Timestamp,
}

/// Per-user staking info within a specific pool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserStakeInfo {
    /// Amount of LP tokens the user has staked.
    pub amount: Amount,
    /// Reward debt: used to calculate pending rewards correctly.
    /// reward_debt = amount * acc_reward_per_share / ACC_REWARD_PRECISION at the time of deposit.
    pub reward_debt: u128,
}

impl UserStakeInfo {
    fn new() -> Self {
        Self {
            amount: Amount::ZERO,
            reward_debt: 0,
        }
    }
}

/// Manages multiple farming pools and distributes rewards across them.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FarmManager {
    /// All registered farming pools.
    pub pools: Vec<FarmPool>,
    /// Per-user stake info: (pool_index, user_address) -> UserStakeInfo.
    pub user_info: HashMap<(usize, Address), UserStakeInfo>,
    /// Global reward emission rate (reward tokens per second).
    pub reward_per_second: Amount,
    /// Sum of all allocation points across pools.
    pub total_alloc_point: u64,
}

impl FarmManager {
    pub fn new(reward_per_second: Amount) -> Self {
        Self {
            pools: Vec::new(),
            user_info: HashMap::new(),
            reward_per_second,
            total_alloc_point: 0,
        }
    }

    /// Add a new farming pool. Returns its index.
    pub fn add_pool(
        &mut self,
        lp_token: AssetId,
        alloc_point: u64,
        timestamp: Timestamp,
    ) -> usize {
        let pool_id = PoolId::new(self.pools.len() as u64);
        self.pools.push(FarmPool {
            pool_id,
            lp_token,
            alloc_point,
            acc_reward_per_share: 0,
            total_staked: Amount::ZERO,
            last_reward_time: timestamp,
        });
        self.total_alloc_point += alloc_point;
        self.pools.len() - 1
    }

    /// Update a pool's accumulated reward per share based on elapsed time.
    pub fn update_pool(&mut self, pool_index: usize, timestamp: Timestamp) -> DefiResult<()> {
        let pool = self
            .pools
            .get_mut(pool_index)
            .ok_or(DefiError::InvalidPool {
                pool_id: PoolId::new(pool_index as u64),
            })?;

        if timestamp <= pool.last_reward_time {
            return Ok(());
        }

        if pool.total_staked.is_zero() || self.total_alloc_point == 0 {
            pool.last_reward_time = timestamp;
            return Ok(());
        }

        let elapsed = timestamp.elapsed_since(pool.last_reward_time) as u128;
        let pool_alloc = pool.alloc_point as u128;
        let total_alloc = self.total_alloc_point as u128;
        let rps = self.reward_per_second.0;

        // reward = elapsed * reward_per_second * pool_alloc / total_alloc
        let reward = elapsed
            .checked_mul(rps)
            .and_then(|v| v.checked_mul(pool_alloc))
            .and_then(|v| v.checked_div(total_alloc))
            .ok_or(DefiError::Overflow {
                operation: "update_pool reward calculation".into(),
            })?;

        // acc_reward_per_share += reward * ACC_REWARD_PRECISION / total_staked
        let acc_increment = reward
            .checked_mul(ACC_REWARD_PRECISION)
            .and_then(|v| v.checked_div(pool.total_staked.0))
            .ok_or(DefiError::Overflow {
                operation: "update_pool acc_reward".into(),
            })?;

        pool.acc_reward_per_share = pool
            .acc_reward_per_share
            .checked_add(acc_increment)
            .ok_or(DefiError::Overflow {
                operation: "update_pool acc_reward sum".into(),
            })?;

        pool.last_reward_time = timestamp;
        Ok(())
    }

    /// Deposit (stake) LP tokens into a farming pool.
    pub fn deposit(
        &mut self,
        user: Address,
        pool_index: usize,
        amount: Amount,
        timestamp: Timestamp,
    ) -> DefiResult<Amount> {
        self.update_pool(pool_index, timestamp)?;

        let pool = self.pools.get(pool_index).ok_or(DefiError::InvalidPool {
            pool_id: PoolId::new(pool_index as u64),
        })?;
        let acc = pool.acc_reward_per_share;
        let key = (pool_index, user);
        let info = self.user_info.entry(key).or_insert_with(UserStakeInfo::new);

        // Calculate pending rewards before modifying the stake.
        let pending = if info.amount.is_zero() {
            Amount::ZERO
        } else {
            let accumulated = info
                .amount
                .0
                .checked_mul(acc)
                .and_then(|v| v.checked_div(ACC_REWARD_PRECISION))
                .unwrap_or(0);
            Amount(accumulated.saturating_sub(info.reward_debt))
        };

        // Increase stake.
        info.amount = info.amount.checked_add(amount).ok_or(DefiError::Overflow {
            operation: "farm deposit".into(),
        })?;

        // Update reward debt.
        info.reward_debt = info
            .amount
            .0
            .checked_mul(acc)
            .and_then(|v| v.checked_div(ACC_REWARD_PRECISION))
            .ok_or(DefiError::Overflow {
                operation: "farm deposit reward_debt".into(),
            })?;

        // Update pool total.
        let pool = self.pools.get_mut(pool_index).ok_or(DefiError::InvalidPool {
            pool_id: PoolId::new(pool_index as u64),
        })?;
        pool.total_staked = pool
            .total_staked
            .checked_add(amount)
            .ok_or(DefiError::Overflow {
                operation: "farm deposit total_staked".into(),
            })?;

        Ok(pending)
    }

    /// Withdraw (unstake) LP tokens from a farming pool and harvest rewards.
    pub fn withdraw(
        &mut self,
        user: Address,
        pool_index: usize,
        amount: Amount,
        timestamp: Timestamp,
    ) -> DefiResult<Amount> {
        self.update_pool(pool_index, timestamp)?;

        let pool = self.pools.get(pool_index).ok_or(DefiError::InvalidPool {
            pool_id: PoolId::new(pool_index as u64),
        })?;
        let acc = pool.acc_reward_per_share;
        let key = (pool_index, user);
        let info = self.user_info.get_mut(&key).ok_or(DefiError::Unauthorized {
            reason: "no stake found for user".into(),
        })?;

        if amount > info.amount {
            return Err(DefiError::InsufficientBalance {
                required: amount,
                available: info.amount,
            });
        }

        // Calculate pending rewards.
        let accumulated = info
            .amount
            .0
            .checked_mul(acc)
            .and_then(|v| v.checked_div(ACC_REWARD_PRECISION))
            .unwrap_or(0);
        let pending = Amount(accumulated.saturating_sub(info.reward_debt));

        // Decrease stake.
        info.amount = info.amount.checked_sub(amount).ok_or(DefiError::Overflow {
            operation: "farm withdraw".into(),
        })?;

        // Update reward debt.
        info.reward_debt = info
            .amount
            .0
            .checked_mul(acc)
            .and_then(|v| v.checked_div(ACC_REWARD_PRECISION))
            .ok_or(DefiError::Overflow {
                operation: "farm withdraw reward_debt".into(),
            })?;

        // Update pool total.
        let pool = self.pools.get_mut(pool_index).ok_or(DefiError::InvalidPool {
            pool_id: PoolId::new(pool_index as u64),
        })?;
        pool.total_staked = pool
            .total_staked
            .checked_sub(amount)
            .ok_or(DefiError::Overflow {
                operation: "farm withdraw total_staked".into(),
            })?;

        Ok(pending)
    }

    /// Get pending rewards for a user in a pool without modifying state.
    pub fn pending_rewards(
        &self,
        user: &Address,
        pool_index: usize,
        timestamp: Timestamp,
    ) -> DefiResult<Amount> {
        let pool = self.pools.get(pool_index).ok_or(DefiError::InvalidPool {
            pool_id: PoolId::new(pool_index as u64),
        })?;

        let key = (pool_index, *user);
        let info = match self.user_info.get(&key) {
            Some(info) => info,
            None => return Ok(Amount::ZERO),
        };

        // Simulate pool update to get current acc_reward_per_share.
        let mut acc = pool.acc_reward_per_share;
        if timestamp > pool.last_reward_time
            && !pool.total_staked.is_zero()
            && self.total_alloc_point > 0
        {
            let elapsed = timestamp.elapsed_since(pool.last_reward_time) as u128;
            let pool_alloc = pool.alloc_point as u128;
            let total_alloc = self.total_alloc_point as u128;
            let rps = self.reward_per_second.0;

            let reward = elapsed
                .saturating_mul(rps)
                .saturating_mul(pool_alloc)
                / total_alloc;

            let acc_increment = reward
                .saturating_mul(ACC_REWARD_PRECISION)
                / pool.total_staked.0;

            acc = acc.saturating_add(acc_increment);
        }

        let accumulated = info.amount.0.saturating_mul(acc) / ACC_REWARD_PRECISION;
        let pending = accumulated.saturating_sub(info.reward_debt);

        Ok(Amount(pending))
    }
}

impl Default for FarmManager {
    fn default() -> Self {
        Self::new(Amount::ZERO)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(n: u8) -> Address {
        let mut bytes = [0u8; 32];
        bytes[0] = n;
        Address::from_bytes(bytes)
    }

    fn ts(secs: u64) -> Timestamp {
        Timestamp::new(secs)
    }

    #[test]
    fn test_add_pool() {
        let mut fm = FarmManager::new(Amount::new(100));
        let idx = fm.add_pool(AssetId::new("LP-A"), 100, ts(0));
        assert_eq!(idx, 0);
        assert_eq!(fm.pools.len(), 1);
        assert_eq!(fm.total_alloc_point, 100);
    }

    #[test]
    fn test_deposit_initial() {
        let mut fm = FarmManager::new(Amount::new(100));
        fm.add_pool(AssetId::new("LP"), 100, ts(0));
        let pending = fm.deposit(addr(1), 0, Amount::new(1000), ts(0)).unwrap();
        assert_eq!(pending, Amount::ZERO); // No pending on first deposit
        assert_eq!(fm.pools[0].total_staked, Amount::new(1000));
    }

    #[test]
    fn test_reward_accrual() {
        let mut fm = FarmManager::new(Amount::new(100)); // 100 tokens/sec
        fm.add_pool(AssetId::new("LP"), 100, ts(0));
        fm.deposit(addr(1), 0, Amount::new(1000), ts(0)).unwrap();

        // After 10 seconds: 100 * 10 = 1000 tokens for the pool
        let pending = fm.pending_rewards(&addr(1), 0, ts(10)).unwrap();
        assert_eq!(pending, Amount::new(1000));
    }

    #[test]
    fn test_withdraw_harvests_rewards() {
        let mut fm = FarmManager::new(Amount::new(100));
        fm.add_pool(AssetId::new("LP"), 100, ts(0));
        fm.deposit(addr(1), 0, Amount::new(1000), ts(0)).unwrap();

        // Withdraw after 10 seconds
        let rewards = fm.withdraw(addr(1), 0, Amount::new(500), ts(10)).unwrap();
        assert_eq!(rewards, Amount::new(1000)); // 100 * 10
        assert_eq!(fm.pools[0].total_staked, Amount::new(500));
    }

    #[test]
    fn test_withdraw_more_than_staked_fails() {
        let mut fm = FarmManager::new(Amount::new(100));
        fm.add_pool(AssetId::new("LP"), 100, ts(0));
        fm.deposit(addr(1), 0, Amount::new(1000), ts(0)).unwrap();
        let result = fm.withdraw(addr(1), 0, Amount::new(2000), ts(10));
        assert!(result.is_err());
    }

    #[test]
    fn test_two_users_split_rewards() {
        let mut fm = FarmManager::new(Amount::new(100));
        fm.add_pool(AssetId::new("LP"), 100, ts(0));

        // User 1 deposits at t=0
        fm.deposit(addr(1), 0, Amount::new(1000), ts(0)).unwrap();
        // User 2 deposits at t=5
        fm.deposit(addr(2), 0, Amount::new(1000), ts(5)).unwrap();

        // At t=10: user 1 had sole access for 5 seconds (500 tokens), then split for 5 seconds
        // User 1: 500 + 250 = 750, User 2: 250
        let p1 = fm.pending_rewards(&addr(1), 0, ts(10)).unwrap();
        let p2 = fm.pending_rewards(&addr(2), 0, ts(10)).unwrap();
        assert_eq!(p1, Amount::new(750));
        assert_eq!(p2, Amount::new(250));
    }

    #[test]
    fn test_multiple_pools_allocation() {
        let mut fm = FarmManager::new(Amount::new(100));
        fm.add_pool(AssetId::new("LP-A"), 75, ts(0));  // 75%
        fm.add_pool(AssetId::new("LP-B"), 25, ts(0));  // 25%

        fm.deposit(addr(1), 0, Amount::new(1000), ts(0)).unwrap();
        fm.deposit(addr(2), 1, Amount::new(1000), ts(0)).unwrap();

        // After 10 seconds: pool 0 gets 100 * 10 * 75 / 100 = 750
        // pool 1 gets 100 * 10 * 25 / 100 = 250
        let p1 = fm.pending_rewards(&addr(1), 0, ts(10)).unwrap();
        let p2 = fm.pending_rewards(&addr(2), 1, ts(10)).unwrap();
        assert_eq!(p1, Amount::new(750));
        assert_eq!(p2, Amount::new(250));
    }

    #[test]
    fn test_pending_rewards_no_stake() {
        let mut fm = FarmManager::new(Amount::new(100));
        fm.add_pool(AssetId::new("LP"), 100, ts(0));
        let pending = fm.pending_rewards(&addr(99), 0, ts(10)).unwrap();
        assert_eq!(pending, Amount::ZERO);
    }
}
