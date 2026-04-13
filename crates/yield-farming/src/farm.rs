use crate::escrow::{
    EscrowSettlement, EscrowTelemetry, QualityOutcomeSignalSource, QualityVerificationOutcome,
    RewardEscrowBook, RewardEscrowPolicy,
};
use crate::utility_hooks::UtilityRewardHook;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use wrfcoin_defi_common::error::{DefiError, DefiResult};
use wrfcoin_defi_common::types::{
    Address, Amount, AssetId, PoolId, Timestamp, UtilitySignalSnapshot,
};

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
    /// Accumulated baseline reward per share before utility weighting.
    pub acc_base_reward_per_share: u128,
    /// Total amount of LP tokens staked in this pool.
    pub total_staked: Amount,
    /// Timestamp of the last reward distribution update.
    pub last_reward_time: Timestamp,
    /// Hard ceiling on emitted rewards per second for this pool.
    pub max_reward_per_second: Amount,
}

/// Per-user staking info within a specific pool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserStakeInfo {
    /// Amount of LP tokens the user has staked.
    pub amount: Amount,
    /// Baseline reward debt before utility weighting.
    pub base_reward_debt: u128,
    /// Reward debt: used to calculate pending rewards correctly.
    /// reward_debt = amount * acc_reward_per_share / ACC_REWARD_PRECISION at the time of deposit.
    pub reward_debt: u128,
}

impl UserStakeInfo {
    fn new() -> Self {
        Self {
            amount: Amount::ZERO,
            base_reward_debt: 0,
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
    /// Hard ceiling on global rewards emitted per second.
    pub max_reward_per_second: Amount,
    /// Sum of all allocation points across pools.
    pub total_alloc_point: u64,
    /// Utility-linked reward hook configuration and logic.
    pub utility_reward_hook: UtilityRewardHook,
    /// Utility signal snapshot per pool index.
    pub pool_utility_signals: HashMap<usize, UtilitySignalSnapshot>,
    /// Optional tighter per-address multiplier ceilings in basis points.
    pub user_multiplier_limits_bps: HashMap<Address, u16>,
    /// Reward escrow state for quality/utility verification-gated payouts.
    pub reward_escrow: RewardEscrowBook,
    /// Derivative tokens blocked from being used as farm pool LP tokens.
    /// Prevents reward stacking amplification where derivative tokens (e.g. stWRF)
    /// earn both underlying staking rewards and farm rewards simultaneously.
    pub blocked_lp_tokens: HashSet<AssetId>,
}

impl FarmManager {
    pub fn new(reward_per_second: Amount) -> Self {
        Self {
            pools: Vec::new(),
            user_info: HashMap::new(),
            reward_per_second,
            max_reward_per_second: reward_per_second,
            total_alloc_point: 0,
            utility_reward_hook: UtilityRewardHook::default(),
            pool_utility_signals: HashMap::new(),
            user_multiplier_limits_bps: HashMap::new(),
            reward_escrow: RewardEscrowBook::default(),
            blocked_lp_tokens: HashSet::new(),
        }
    }

    /// Block a derivative token from being used as a farm pool LP token.
    /// Prevents reward stacking amplification (issue #47).
    pub fn block_derivative_token(&mut self, asset_id: AssetId) {
        self.blocked_lp_tokens.insert(asset_id);
    }

    /// Remove a token from the derivative blocklist.
    pub fn unblock_derivative_token(&mut self, asset_id: &AssetId) {
        self.blocked_lp_tokens.remove(asset_id);
    }

    /// Add a new farming pool. Returns its index.
    /// Rejects derivative tokens that are on the blocklist.
    pub fn add_pool(
        &mut self,
        lp_token: AssetId,
        alloc_point: u64,
        timestamp: Timestamp,
    ) -> DefiResult<usize> {
        if self.blocked_lp_tokens.contains(&lp_token) {
            return Err(DefiError::InvalidAsset { asset_id: lp_token });
        }
        let pool_id = PoolId::new(self.pools.len() as u64);
        self.pools.push(FarmPool {
            pool_id,
            lp_token,
            alloc_point,
            acc_reward_per_share: 0,
            acc_base_reward_per_share: 0,
            total_staked: Amount::ZERO,
            last_reward_time: timestamp,
            max_reward_per_second: self.max_reward_per_second,
        });
        self.total_alloc_point += alloc_point;
        let pool_index = self.pools.len() - 1;
        self.pool_utility_signals.entry(pool_index).or_default();
        Ok(pool_index)
    }

    /// Update utility signal snapshot used by reward hooks for a pool.
    pub fn set_pool_utility_signal(
        &mut self,
        pool_index: usize,
        signal: UtilitySignalSnapshot,
    ) -> DefiResult<()> {
        if pool_index >= self.pools.len() {
            return Err(DefiError::InvalidPool {
                pool_id: PoolId::new(pool_index as u64),
            });
        }
        self.pool_utility_signals
            .insert(pool_index, signal.clamped());
        Ok(())
    }

    pub fn pool_utility_signal(&self, pool_index: usize) -> DefiResult<UtilitySignalSnapshot> {
        if pool_index >= self.pools.len() {
            return Err(DefiError::InvalidPool {
                pool_id: PoolId::new(pool_index as u64),
            });
        }
        Ok(self.signal_for_pool(pool_index))
    }

    fn signal_for_pool(&self, pool_index: usize) -> UtilitySignalSnapshot {
        self.pool_utility_signals
            .get(&pool_index)
            .copied()
            .unwrap_or_default()
    }

    pub fn set_reward_escrow_policy(&mut self, policy: RewardEscrowPolicy) -> DefiResult<()> {
        self.reward_escrow.set_policy(policy)
    }

    pub fn reward_escrow_telemetry(&self) -> EscrowTelemetry {
        self.reward_escrow.telemetry()
    }

    pub fn set_global_emission_cap(&mut self, max_reward_per_second: Amount) {
        self.max_reward_per_second = max_reward_per_second;
    }

    pub fn set_pool_emission_cap(
        &mut self,
        pool_index: usize,
        max_reward_per_second: Amount,
    ) -> DefiResult<()> {
        let pool = self
            .pools
            .get_mut(pool_index)
            .ok_or(DefiError::InvalidPool {
                pool_id: PoolId::new(pool_index as u64),
            })?;
        pool.max_reward_per_second = max_reward_per_second;
        Ok(())
    }

    pub fn set_user_multiplier_limit(
        &mut self,
        user: Address,
        multiplier_limit_bps: u16,
    ) -> DefiResult<()> {
        self.utility_reward_hook.config.validate()?;
        if multiplier_limit_bps < self.utility_reward_hook.config.neutral_multiplier_bps
            || multiplier_limit_bps
                > self
                    .utility_reward_hook
                    .config
                    .per_address_multiplier_limit_bps
        {
            return Err(DefiError::InvalidAmount {
                reason: "user multiplier limit must be between neutral and configured cap".into(),
            });
        }
        self.user_multiplier_limits_bps
            .insert(user, multiplier_limit_bps);
        Ok(())
    }

    pub fn escrowed_reward_count(&self) -> usize {
        self.reward_escrow.escrowed_count()
    }

    pub fn escrowed_reward(&self, escrow_id: u64) -> DefiResult<&crate::escrow::EscrowedReward> {
        self.reward_escrow
            .escrowed_reward(escrow_id)
            .ok_or(DefiError::InvalidAmount {
                reason: format!("unknown escrow_id {escrow_id}"),
            })
    }

    fn harvestable_reward(
        &mut self,
        user: Address,
        pool_index: usize,
        timestamp: Timestamp,
    ) -> DefiResult<(PoolId, Amount)> {
        self.update_pool(pool_index, timestamp)?;

        let (pool_id, acc_base_reward_per_share, acc_reward_per_share) = self
            .pools
            .get(pool_index)
            .map(|pool| {
                (
                    pool.pool_id,
                    pool.acc_base_reward_per_share,
                    pool.acc_reward_per_share,
                )
            })
            .ok_or(DefiError::InvalidPool {
                pool_id: PoolId::new(pool_index as u64),
            })?;
        let reward_hook = self.utility_reward_hook;
        let user_multiplier_limit_bps = self.effective_user_multiplier_limit_bps(&user);

        let key = (pool_index, user);
        let info = self
            .user_info
            .get_mut(&key)
            .ok_or(DefiError::Unauthorized {
                reason: "no stake found for user".into(),
            })?;

        let accumulated_base = info
            .amount
            .0
            .checked_mul(acc_base_reward_per_share)
            .and_then(|value| value.checked_div(ACC_REWARD_PRECISION))
            .ok_or(DefiError::Overflow {
                operation: "farm harvest accumulated base".into(),
            })?;
        let accumulated = info
            .amount
            .0
            .checked_mul(acc_reward_per_share)
            .and_then(|value| value.checked_div(ACC_REWARD_PRECISION))
            .ok_or(DefiError::Overflow {
                operation: "farm harvest accumulated".into(),
            })?;
        let pending = reward_hook.cap_reward(
            Amount(accumulated_base.saturating_sub(info.base_reward_debt)),
            Amount(accumulated.saturating_sub(info.reward_debt)),
            user_multiplier_limit_bps,
        )?;
        info.base_reward_debt = accumulated_base;
        info.reward_debt = accumulated;

        Ok((pool_id, pending))
    }

    fn effective_user_multiplier_limit_bps(&self, user: &Address) -> u16 {
        self.user_multiplier_limits_bps
            .get(user)
            .copied()
            .unwrap_or(
                self.utility_reward_hook
                    .config
                    .per_address_multiplier_limit_bps,
            )
            .min(
                self.utility_reward_hook
                    .config
                    .per_address_multiplier_limit_bps,
            )
            .max(self.utility_reward_hook.config.neutral_multiplier_bps)
    }

    fn pool_reward_for_elapsed(
        &self,
        pool_index: usize,
        elapsed: u64,
    ) -> DefiResult<(Amount, Amount)> {
        let pool = self.pools.get(pool_index).ok_or(DefiError::InvalidPool {
            pool_id: PoolId::new(pool_index as u64),
        })?;
        let effective_global_rps = self.reward_per_second.min(self.max_reward_per_second).0;
        let base_reward = (elapsed as u128)
            .checked_mul(effective_global_rps)
            .and_then(|value| value.checked_mul(pool.alloc_point as u128))
            .and_then(|value| value.checked_div(self.total_alloc_point as u128))
            .ok_or(DefiError::Overflow {
                operation: "update_pool reward calculation".into(),
            })?;
        let pool_cap = (elapsed as u128)
            .checked_mul(pool.max_reward_per_second.0)
            .ok_or(DefiError::Overflow {
                operation: "update_pool pool emission cap".into(),
            })?;
        let base_reward = Amount::new(base_reward.min(pool_cap));
        let weighted_reward = self
            .utility_reward_hook
            .apply(base_reward, &self.signal_for_pool(pool_index))?;
        Ok((base_reward, weighted_reward))
    }

    /// Harvest rewards and route payout through escrow policy using an explicit verification outcome.
    pub fn harvest_with_outcome(
        &mut self,
        user: Address,
        pool_index: usize,
        timestamp: Timestamp,
        outcome: QualityVerificationOutcome,
    ) -> DefiResult<EscrowSettlement> {
        let (pool_id, pending) = self.harvestable_reward(user, pool_index, timestamp)?;
        self.reward_escrow
            .record_reward(user, pool_id, pending, timestamp, outcome)
    }

    /// Harvest rewards and fetch verification outcome from a signal source.
    pub fn harvest_with_signal_source<S: QualityOutcomeSignalSource>(
        &mut self,
        user: Address,
        pool_index: usize,
        timestamp: Timestamp,
        signal_source: &S,
    ) -> DefiResult<EscrowSettlement> {
        let (pool_id, pending) = self.harvestable_reward(user, pool_index, timestamp)?;
        let outcome = signal_source.quality_outcome_for_reward(user, pool_id, pending, timestamp);
        self.reward_escrow
            .record_reward(user, pool_id, pending, timestamp, outcome)
    }

    pub fn settle_escrowed_reward(
        &mut self,
        escrow_id: u64,
        outcome: QualityVerificationOutcome,
        settled_at: Timestamp,
    ) -> DefiResult<EscrowSettlement> {
        self.reward_escrow
            .settle_escrow(escrow_id, outcome, settled_at)
    }

    /// Update a pool's accumulated reward per share based on elapsed time.
    pub fn update_pool(&mut self, pool_index: usize, timestamp: Timestamp) -> DefiResult<()> {
        let pool = self.pools.get(pool_index).ok_or(DefiError::InvalidPool {
            pool_id: PoolId::new(pool_index as u64),
        })?;

        if timestamp <= pool.last_reward_time {
            return Ok(());
        }

        if pool.total_staked.is_zero() || self.total_alloc_point == 0 {
            let pool = self
                .pools
                .get_mut(pool_index)
                .ok_or(DefiError::InvalidPool {
                    pool_id: PoolId::new(pool_index as u64),
                })?;
            pool.last_reward_time = timestamp;
            return Ok(());
        }

        let elapsed = timestamp.elapsed_since(pool.last_reward_time);
        let (base_reward, reward) = self.pool_reward_for_elapsed(pool_index, elapsed)?;

        // acc_reward_per_share += reward * ACC_REWARD_PRECISION / total_staked
        let base_acc_increment = base_reward
            .0
            .checked_mul(ACC_REWARD_PRECISION)
            .and_then(|v| v.checked_div(pool.total_staked.0))
            .ok_or(DefiError::Overflow {
                operation: "update_pool base acc_reward".into(),
            })?;
        let acc_increment = reward
            .0
            .checked_mul(ACC_REWARD_PRECISION)
            .and_then(|v| v.checked_div(pool.total_staked.0))
            .ok_or(DefiError::Overflow {
                operation: "update_pool acc_reward".into(),
            })?;

        let pool = self
            .pools
            .get_mut(pool_index)
            .ok_or(DefiError::InvalidPool {
                pool_id: PoolId::new(pool_index as u64),
            })?;
        pool.acc_base_reward_per_share = pool
            .acc_base_reward_per_share
            .checked_add(base_acc_increment)
            .ok_or(DefiError::Overflow {
                operation: "update_pool base acc_reward sum".into(),
            })?;
        pool.acc_reward_per_share =
            pool.acc_reward_per_share
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
        let base_acc = pool.acc_base_reward_per_share;
        let acc = pool.acc_reward_per_share;
        let reward_hook = self.utility_reward_hook;
        let user_multiplier_limit_bps = self.effective_user_multiplier_limit_bps(&user);
        let key = (pool_index, user);
        let info = self.user_info.entry(key).or_insert_with(UserStakeInfo::new);

        // Calculate pending rewards before modifying the stake.
        let pending = if info.amount.is_zero() {
            Amount::ZERO
        } else {
            let accumulated_base = info
                .amount
                .0
                .checked_mul(base_acc)
                .and_then(|v| v.checked_div(ACC_REWARD_PRECISION))
                .unwrap_or(0);
            let accumulated = info
                .amount
                .0
                .checked_mul(acc)
                .and_then(|v| v.checked_div(ACC_REWARD_PRECISION))
                .unwrap_or(0);
            reward_hook.cap_reward(
                Amount(accumulated_base.saturating_sub(info.base_reward_debt)),
                Amount(accumulated.saturating_sub(info.reward_debt)),
                user_multiplier_limit_bps,
            )?
        };

        // Increase stake.
        info.amount = info.amount.checked_add(amount).ok_or(DefiError::Overflow {
            operation: "farm deposit".into(),
        })?;

        // Update reward debt.
        info.base_reward_debt = info
            .amount
            .0
            .checked_mul(base_acc)
            .and_then(|v| v.checked_div(ACC_REWARD_PRECISION))
            .ok_or(DefiError::Overflow {
                operation: "farm deposit base_reward_debt".into(),
            })?;
        info.reward_debt = info
            .amount
            .0
            .checked_mul(acc)
            .and_then(|v| v.checked_div(ACC_REWARD_PRECISION))
            .ok_or(DefiError::Overflow {
                operation: "farm deposit reward_debt".into(),
            })?;

        // Update pool total.
        let pool = self
            .pools
            .get_mut(pool_index)
            .ok_or(DefiError::InvalidPool {
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
        let base_acc = pool.acc_base_reward_per_share;
        let acc = pool.acc_reward_per_share;
        let reward_hook = self.utility_reward_hook;
        let user_multiplier_limit_bps = self.effective_user_multiplier_limit_bps(&user);
        let key = (pool_index, user);
        let info = self
            .user_info
            .get_mut(&key)
            .ok_or(DefiError::Unauthorized {
                reason: "no stake found for user".into(),
            })?;

        if amount > info.amount {
            return Err(DefiError::InsufficientBalance {
                required: amount,
                available: info.amount,
            });
        }

        // Calculate pending rewards.
        let accumulated_base = info
            .amount
            .0
            .checked_mul(base_acc)
            .and_then(|v| v.checked_div(ACC_REWARD_PRECISION))
            .unwrap_or(0);
        let accumulated = info
            .amount
            .0
            .checked_mul(acc)
            .and_then(|v| v.checked_div(ACC_REWARD_PRECISION))
            .unwrap_or(0);
        let pending = reward_hook.cap_reward(
            Amount(accumulated_base.saturating_sub(info.base_reward_debt)),
            Amount(accumulated.saturating_sub(info.reward_debt)),
            user_multiplier_limit_bps,
        )?;

        // Decrease stake.
        info.amount = info.amount.checked_sub(amount).ok_or(DefiError::Overflow {
            operation: "farm withdraw".into(),
        })?;

        // Update reward debt.
        info.base_reward_debt = info
            .amount
            .0
            .checked_mul(base_acc)
            .and_then(|v| v.checked_div(ACC_REWARD_PRECISION))
            .ok_or(DefiError::Overflow {
                operation: "farm withdraw base_reward_debt".into(),
            })?;
        info.reward_debt = info
            .amount
            .0
            .checked_mul(acc)
            .and_then(|v| v.checked_div(ACC_REWARD_PRECISION))
            .ok_or(DefiError::Overflow {
                operation: "farm withdraw reward_debt".into(),
            })?;

        // Update pool total.
        let pool = self
            .pools
            .get_mut(pool_index)
            .ok_or(DefiError::InvalidPool {
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
        let mut base_acc = pool.acc_base_reward_per_share;
        let mut acc = pool.acc_reward_per_share;
        if timestamp > pool.last_reward_time
            && !pool.total_staked.is_zero()
            && self.total_alloc_point > 0
        {
            let (base_reward, reward) = self.pool_reward_for_elapsed(
                pool_index,
                timestamp.elapsed_since(pool.last_reward_time),
            )?;
            let base_acc_increment =
                base_reward.0.saturating_mul(ACC_REWARD_PRECISION) / pool.total_staked.0;
            let acc_increment = reward.0.saturating_mul(ACC_REWARD_PRECISION) / pool.total_staked.0;

            base_acc = base_acc.saturating_add(base_acc_increment);
            acc = acc.saturating_add(acc_increment);
        }

        let accumulated_base = info.amount.0.saturating_mul(base_acc) / ACC_REWARD_PRECISION;
        let accumulated = info.amount.0.saturating_mul(acc) / ACC_REWARD_PRECISION;
        self.utility_reward_hook.cap_reward(
            Amount(accumulated_base.saturating_sub(info.base_reward_debt)),
            Amount(accumulated.saturating_sub(info.reward_debt)),
            self.effective_user_multiplier_limit_bps(user),
        )
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
    use crate::escrow::{ForfeitReason, QualityVerificationOutcome};
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use wrfcoin_defi_common::types::UtilitySignalSnapshot;

    fn addr(n: u8) -> Address {
        let mut bytes = [0u8; 32];
        bytes[0] = n;
        Address::from_bytes(bytes)
    }

    fn ts(secs: u64) -> Timestamp {
        Timestamp::new(secs)
    }

    struct MockQualitySource {
        outcomes: RefCell<VecDeque<QualityVerificationOutcome>>,
    }

    impl MockQualitySource {
        fn new(outcomes: Vec<QualityVerificationOutcome>) -> Self {
            Self {
                outcomes: RefCell::new(VecDeque::from(outcomes)),
            }
        }
    }

    impl QualityOutcomeSignalSource for MockQualitySource {
        fn quality_outcome_for_reward(
            &self,
            _beneficiary: Address,
            _pool_id: PoolId,
            _reward_amount: Amount,
            _reward_timestamp: Timestamp,
        ) -> QualityVerificationOutcome {
            self.outcomes
                .borrow_mut()
                .pop_front()
                .unwrap_or(QualityVerificationOutcome::Pending)
        }
    }

    #[test]
    fn test_add_pool() {
        let mut fm = FarmManager::new(Amount::new(100));
        let idx = fm.add_pool(AssetId::new("LP-A"), 100, ts(0)).unwrap();
        assert_eq!(idx, 0);
        assert_eq!(fm.pools.len(), 1);
        assert_eq!(fm.total_alloc_point, 100);
    }

    #[test]
    fn test_deposit_initial() {
        let mut fm = FarmManager::new(Amount::new(100));
        fm.add_pool(AssetId::new("LP"), 100, ts(0)).unwrap();
        let pending = fm.deposit(addr(1), 0, Amount::new(1000), ts(0)).unwrap();
        assert_eq!(pending, Amount::ZERO); // No pending on first deposit
        assert_eq!(fm.pools[0].total_staked, Amount::new(1000));
    }

    #[test]
    fn test_reward_accrual() {
        let mut fm = FarmManager::new(Amount::new(100)); // 100 tokens/sec
        fm.add_pool(AssetId::new("LP"), 100, ts(0)).unwrap();
        fm.deposit(addr(1), 0, Amount::new(1000), ts(0)).unwrap();

        // After 10 seconds: 100 * 10 = 1000 tokens for the pool
        let pending = fm.pending_rewards(&addr(1), 0, ts(10)).unwrap();
        assert_eq!(pending, Amount::new(1000));
    }

    #[test]
    fn test_withdraw_harvests_rewards() {
        let mut fm = FarmManager::new(Amount::new(100));
        fm.add_pool(AssetId::new("LP"), 100, ts(0)).unwrap();
        fm.deposit(addr(1), 0, Amount::new(1000), ts(0)).unwrap();

        // Withdraw after 10 seconds
        let rewards = fm.withdraw(addr(1), 0, Amount::new(500), ts(10)).unwrap();
        assert_eq!(rewards, Amount::new(1000)); // 100 * 10
        assert_eq!(fm.pools[0].total_staked, Amount::new(500));
    }

    #[test]
    fn test_withdraw_more_than_staked_fails() {
        let mut fm = FarmManager::new(Amount::new(100));
        fm.add_pool(AssetId::new("LP"), 100, ts(0)).unwrap();
        fm.deposit(addr(1), 0, Amount::new(1000), ts(0)).unwrap();
        let result = fm.withdraw(addr(1), 0, Amount::new(2000), ts(10));
        assert!(result.is_err());
    }

    #[test]
    fn test_two_users_split_rewards() {
        let mut fm = FarmManager::new(Amount::new(100));
        fm.add_pool(AssetId::new("LP"), 100, ts(0)).unwrap();

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
        fm.add_pool(AssetId::new("LP-A"), 75, ts(0)).unwrap(); // 75%
        fm.add_pool(AssetId::new("LP-B"), 25, ts(0)).unwrap(); // 25%

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
        fm.add_pool(AssetId::new("LP"), 100, ts(0)).unwrap();
        let pending = fm.pending_rewards(&addr(99), 0, ts(10)).unwrap();
        assert_eq!(pending, Amount::ZERO);
    }

    #[test]
    fn test_utility_signal_penalizes_low_verified_usefulness() {
        let mut fm = FarmManager::new(Amount::new(100));
        fm.add_pool(AssetId::new("LP"), 100, ts(0)).unwrap();
        fm.set_pool_utility_signal(
            0,
            UtilitySignalSnapshot {
                forecast_usefulness_bps: 2_000,
                report_usefulness_bps: 2_000,
                verification_confidence_bps: 10_000,
                verified_events: 1,
                unverified_events: 9,
            },
        )
        .unwrap();
        fm.deposit(addr(1), 0, Amount::new(1000), ts(0)).unwrap();

        let pending = fm.pending_rewards(&addr(1), 0, ts(10)).unwrap();
        assert!(pending < Amount::new(1000));
    }

    #[test]
    fn test_utility_signal_boosts_high_verified_usefulness() {
        let mut fm = FarmManager::new(Amount::new(100));
        fm.add_pool(AssetId::new("LP"), 100, ts(0)).unwrap();
        fm.set_pool_utility_signal(
            0,
            UtilitySignalSnapshot {
                forecast_usefulness_bps: 9_000,
                report_usefulness_bps: 9_500,
                verification_confidence_bps: 10_000,
                verified_events: 9,
                unverified_events: 1,
            },
        )
        .unwrap();
        fm.deposit(addr(1), 0, Amount::new(1000), ts(0)).unwrap();

        let pending = fm.pending_rewards(&addr(1), 0, ts(10)).unwrap();
        assert!(pending > Amount::new(1000));
    }

    #[test]
    fn test_global_emission_cap_clamps_rewards() {
        let mut fm = FarmManager::new(Amount::new(100));
        fm.set_global_emission_cap(Amount::new(60));
        fm.add_pool(AssetId::new("LP"), 100, ts(0)).unwrap();
        fm.deposit(addr(1), 0, Amount::new(1000), ts(0)).unwrap();

        let pending = fm.pending_rewards(&addr(1), 0, ts(10)).unwrap();
        assert_eq!(pending, Amount::new(600));
    }

    #[test]
    fn test_pool_emission_cap_clamps_rewards() {
        let mut fm = FarmManager::new(Amount::new(100));
        fm.add_pool(AssetId::new("LP"), 100, ts(0)).unwrap();
        fm.set_pool_emission_cap(0, Amount::new(30)).unwrap();
        fm.deposit(addr(1), 0, Amount::new(1000), ts(0)).unwrap();

        let pending = fm.pending_rewards(&addr(1), 0, ts(10)).unwrap();
        assert_eq!(pending, Amount::new(300));
    }

    #[test]
    fn test_user_multiplier_limit_caps_boosted_rewards() {
        let mut fm = FarmManager::new(Amount::new(100));
        fm.add_pool(AssetId::new("LP"), 100, ts(0)).unwrap();
        fm.set_user_multiplier_limit(addr(1), 11_000).unwrap();
        fm.set_pool_utility_signal(
            0,
            UtilitySignalSnapshot {
                forecast_usefulness_bps: 9_000,
                report_usefulness_bps: 9_500,
                verification_confidence_bps: 10_000,
                verified_events: 9,
                unverified_events: 1,
            },
        )
        .unwrap();
        fm.deposit(addr(1), 0, Amount::new(1000), ts(0)).unwrap();

        let pending = fm.pending_rewards(&addr(1), 0, ts(10)).unwrap();
        assert_eq!(pending, Amount::new(1100));
    }

    #[test]
    fn test_harvest_holds_until_quality_verification_settles() {
        let mut fm = FarmManager::new(Amount::new(100));
        fm.add_pool(AssetId::new("LP"), 100, ts(0)).unwrap();
        fm.deposit(addr(1), 0, Amount::new(1000), ts(0)).unwrap();

        let source = MockQualitySource::new(vec![QualityVerificationOutcome::Pending]);
        let first = fm
            .harvest_with_signal_source(addr(1), 0, ts(10), &source)
            .unwrap();

        let escrow_id = match first {
            EscrowSettlement::Held(entry) => {
                assert_eq!(entry.amount, Amount::new(1000));
                entry.escrow_id
            }
            _ => panic!("expected pending outcome to escrow rewards"),
        };
        assert_eq!(fm.escrowed_reward_count(), 1);

        let settled = fm
            .settle_escrowed_reward(
                escrow_id,
                QualityVerificationOutcome::Verified {
                    quality_score_bps: 7_500,
                },
                ts(12),
            )
            .unwrap();
        match settled {
            EscrowSettlement::Released {
                amount,
                escrow_id: Some(id),
            } => {
                assert_eq!(amount, Amount::new(1000));
                assert_eq!(id, escrow_id);
            }
            _ => panic!("expected escrowed reward to be released"),
        }
        assert_eq!(fm.escrowed_reward_count(), 0);
        assert_eq!(
            fm.reward_escrow_telemetry()
                .released_after_verification_share_bps(),
            10_000
        );
    }

    #[test]
    fn test_harvest_forfeits_low_quality_outcomes() {
        let mut fm = FarmManager::new(Amount::new(100));
        fm.add_pool(AssetId::new("LP"), 100, ts(0)).unwrap();
        fm.deposit(addr(1), 0, Amount::new(1000), ts(0)).unwrap();

        let outcome = QualityVerificationOutcome::Verified {
            quality_score_bps: 4_000,
        };
        let settlement = fm
            .harvest_with_outcome(addr(1), 0, ts(10), outcome)
            .unwrap();

        match settlement {
            EscrowSettlement::Forfeited {
                amount,
                reason: ForfeitReason::LowQuality,
                ..
            } => assert_eq!(amount, Amount::new(1000)),
            _ => panic!("expected low-quality rewards to be forfeited"),
        }
    }

    #[test]
    fn test_harvest_forfeits_fraudulent_outcomes() {
        let mut fm = FarmManager::new(Amount::new(100));
        fm.add_pool(AssetId::new("LP"), 100, ts(0)).unwrap();
        fm.deposit(addr(1), 0, Amount::new(1000), ts(0)).unwrap();

        let settlement = fm
            .harvest_with_outcome(
                addr(1),
                0,
                ts(10),
                QualityVerificationOutcome::FraudDetected,
            )
            .unwrap();
        match settlement {
            EscrowSettlement::Forfeited {
                amount,
                reason: ForfeitReason::FraudDetected,
                ..
            } => assert_eq!(amount, Amount::new(1000)),
            _ => panic!("expected fraudulent rewards to be forfeited"),
        }
    }

    // ── derivative token blocklist (issue #47) ───────────────────────────────

    #[test]
    fn blocked_derivative_token_rejects_pool_creation() {
        let mut fm = FarmManager::new(Amount::new(100));
        fm.block_derivative_token(AssetId::new("stWRF"));

        let result = fm.add_pool(AssetId::new("stWRF"), 100, ts(0));
        assert!(matches!(result, Err(DefiError::InvalidAsset { .. })));
        assert_eq!(fm.pools.len(), 0);
    }

    #[test]
    fn non_blocked_token_pool_creation_succeeds() {
        let mut fm = FarmManager::new(Amount::new(100));
        fm.block_derivative_token(AssetId::new("stWRF"));

        let idx = fm.add_pool(AssetId::new("WRF"), 100, ts(0)).unwrap();
        assert_eq!(idx, 0);
        assert_eq!(fm.pools.len(), 1);
    }

    #[test]
    fn unblocked_token_allows_pool_creation_again() {
        let mut fm = FarmManager::new(Amount::new(100));
        fm.block_derivative_token(AssetId::new("stWRF"));

        assert!(fm.add_pool(AssetId::new("stWRF"), 100, ts(0)).is_err());

        fm.unblock_derivative_token(&AssetId::new("stWRF"));
        let idx = fm.add_pool(AssetId::new("stWRF"), 100, ts(0)).unwrap();
        assert_eq!(idx, 0);
    }

    #[test]
    fn multiple_derivative_tokens_all_blocked() {
        let mut fm = FarmManager::new(Amount::new(100));
        fm.block_derivative_token(AssetId::new("stWRF"));
        fm.block_derivative_token(AssetId::new("vaultLP-stWRF"));

        assert!(fm.add_pool(AssetId::new("stWRF"), 50, ts(0)).is_err());
        assert!(fm
            .add_pool(AssetId::new("vaultLP-stWRF"), 50, ts(0))
            .is_err());

        // Normal tokens still work
        let idx = fm
            .add_pool(AssetId::new("LP-WRF-USDC"), 100, ts(0))
            .unwrap();
        assert_eq!(idx, 0);
    }

    #[test]
    fn reward_stacking_amplification_prevented() {
        // Prove that the stWRF → farm stacking path is blocked:
        // 1. Block stWRF as a derivative token
        // 2. Attempt to create a stWRF farm pool → fails
        // 3. Without the block, stWRF in a farm would earn both staking APY
        //    AND farm APY, amplifying effective yield beyond intended inflation.
        let mut fm = FarmManager::new(Amount::new(100));
        fm.block_derivative_token(AssetId::new("stWRF"));

        // Cannot create a pool that accepts stWRF deposits
        let err = fm.add_pool(AssetId::new("stWRF"), 100, ts(0)).unwrap_err();
        assert!(
            matches!(err, DefiError::InvalidAsset { ref asset_id } if asset_id.as_str() == "stWRF"),
            "expected InvalidAsset for stWRF, got: {err}"
        );

        // Can still create a pool for normal WRF LP tokens
        fm.add_pool(AssetId::new("LP-WRF"), 100, ts(0)).unwrap();
        fm.deposit(addr(1), 0, Amount::new(1000), ts(0)).unwrap();

        // Normal farm rewards work as expected (no amplification)
        let pending = fm.pending_rewards(&addr(1), 0, ts(10)).unwrap();
        assert_eq!(pending, Amount::new(1000)); // 100 rps * 10s, single-source yield only
    }
}
