//! Liquid staking for WRF — the stWRF rebasing receipt token.
//!
//! # Design
//!
//! Users deposit WRF and receive stWRF shares. The pool tracks:
//! - `total_staked`: total WRF held by the pool (increases with rewards, decreases with slashes)
//! - `total_shares`: total stWRF shares outstanding
//!
//! Exchange rate: `wrf_per_share = total_staked / total_shares`
//!
//! When rewards are distributed, `total_staked` increases while `total_shares` stays constant,
//! so every share is worth more WRF — this is the auto-compounding mechanism.
//!
//! Unstaking is subject to an unbonding delay: shares are burned immediately but the WRF
//! is only claimable after `unlock_height` blocks have passed.
//!
//! Slashing: `slash_validator` reduces `total_staked` by the slash amount, diluting all
//! stWRF holders proportionally (no special per-validator accounting needed for the share rate).

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use wrfcoin_defi_common::error::DefiError;
use wrfcoin_defi_common::types::{Address, Amount, AssetId};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Canonical AssetId for the stWRF receipt token.
pub const STWRF_ASSET_ID: &str = "stWRF";

/// Loan-to-value ratio for stWRF used as lending collateral, in basis points (75%).
pub const STWRF_LTV_BPS: u64 = 7_500;

/// Liquidation threshold for stWRF collateral, in basis points (80%).
pub const STWRF_LIQUIDATION_THRESHOLD_BPS: u64 = 8_000;

/// Maximum total weight across all validators must equal exactly this (10 000 bps = 100%).
pub const TOTAL_WEIGHT_BPS: u64 = 10_000;

/// Precision multiplier used in share/underlying ratio calculations to avoid integer truncation.
pub const PRECISION: u128 = 1_000_000_000_000_000_000; // 1e18

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors specific to the liquid staking module.
#[derive(Debug, Clone, thiserror::Error)]
pub enum LiquidStakingError {
    #[error("liquid staking: {0}")]
    Defi(#[from] DefiError),

    #[error("validator {validator_id} not found")]
    ValidatorNotFound { validator_id: u64 },

    #[error("validator weights must sum to {TOTAL_WEIGHT_BPS} bps, got {actual}")]
    InvalidValidatorWeights { actual: u64 },

    #[error("unbonding entry not yet unlocked (current height {current}, unlock height {unlock})")]
    StillBonding { current: u64, unlock: u64 },

    #[error("no claimable unbonding entries at height {current_height}")]
    NothingToClaim { current_height: u64 },

    #[error("pool has no staked WRF — cannot compute share ratio")]
    EmptyPool,
}

pub type LiquidStakingResult<T> = Result<T, LiquidStakingError>;

// ---------------------------------------------------------------------------
// ValidatorAllocation
// ---------------------------------------------------------------------------

/// A validator and its share of the staking pool's total stake.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidatorAllocation {
    /// Opaque validator identifier (e.g., node public-key hash or index).
    pub validator_id: u64,
    /// Fraction of total stake directed to this validator, in basis points.
    /// All allocations in the pool must sum to `TOTAL_WEIGHT_BPS`.
    pub weight_bps: u64,
}

impl ValidatorAllocation {
    pub fn new(validator_id: u64, weight_bps: u64) -> Self {
        Self {
            validator_id,
            weight_bps,
        }
    }
}

// ---------------------------------------------------------------------------
// UnbondingEntry / UnbondingQueue
// ---------------------------------------------------------------------------

/// A single pending withdrawal entry for one user.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnbondingEntry {
    /// WRF amount that will be returned to the user after unbonding.
    pub amount: Amount,
    /// Block height at which this entry becomes claimable.
    pub unlock_height: u64,
}

/// Per-user FIFO queue of pending unbonding entries.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UnbondingQueue {
    entries: VecDeque<UnbondingEntry>,
}

impl UnbondingQueue {
    pub fn new() -> Self {
        Self::default()
    }

    /// Push a new unbonding entry to the back of the queue.
    pub fn push(&mut self, entry: UnbondingEntry) {
        self.entries.push_back(entry);
    }

    /// Claim all entries that have reached or passed `current_height`.
    ///
    /// Returns the total claimable WRF amount and removes those entries.
    pub fn claim_ready(&mut self, current_height: u64) -> LiquidStakingResult<Amount> {
        let mut total = Amount::ZERO;
        let mut any_ready = false;

        // Process from the front (oldest first).
        while let Some(front) = self.entries.front() {
            if front.unlock_height > current_height {
                break;
            }
            let entry = self.entries.pop_front().unwrap();
            total = total
                .checked_add(entry.amount)
                .ok_or_else(|| LiquidStakingError::Defi(DefiError::Overflow {
                    operation: "unbonding claim sum".into(),
                }))?;
            any_ready = true;
        }

        if !any_ready {
            return Err(LiquidStakingError::NothingToClaim { current_height });
        }

        Ok(total)
    }

    /// Peek at how many entries are pending.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Returns all entries (for inspection / tests).
    pub fn entries(&self) -> &VecDeque<UnbondingEntry> {
        &self.entries
    }
}

// ---------------------------------------------------------------------------
// StWRFToken
// ---------------------------------------------------------------------------

/// Rebasing share token bookkeeping.
///
/// Does NOT maintain per-user balances itself — that is tracked in `LiquidStakingPool.shares`.
/// This struct exists to encapsulate the exchange-rate maths.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StWRFToken {
    /// Total stWRF shares in existence.
    pub total_shares: Amount,
    /// Total WRF held by the pool (including accrued rewards, minus slashes).
    pub total_underlying: Amount,
}

impl StWRFToken {
    pub fn new() -> Self {
        Self {
            total_shares: Amount::ZERO,
            total_underlying: Amount::ZERO,
        }
    }

    /// Convert a WRF deposit to stWRF shares.
    ///
    /// On an empty pool the initial exchange rate is 1:1.
    pub fn wrf_to_shares(&self, wrf_amount: Amount) -> LiquidStakingResult<Amount> {
        if wrf_amount.is_zero() {
            return Ok(Amount::ZERO);
        }
        if self.total_shares.is_zero() || self.total_underlying.is_zero() {
            // Initial stake: 1 WRF = 1 share.
            return Ok(wrf_amount);
        }
        // shares = wrf_amount * total_shares / total_underlying
        // Use PRECISION to avoid rounding to zero for small deposits.
        let shares = wrf_amount
            .0
            .checked_mul(self.total_shares.0)
            .and_then(|v| v.checked_div(self.total_underlying.0))
            .ok_or_else(|| LiquidStakingError::Defi(DefiError::Overflow {
                operation: "wrf_to_shares".into(),
            }))?;
        Ok(Amount::new(shares))
    }

    /// Convert stWRF shares to WRF (for redemption / collateral valuation).
    pub fn shares_to_wrf(&self, shares: Amount) -> LiquidStakingResult<Amount> {
        if shares.is_zero() {
            return Ok(Amount::ZERO);
        }
        if self.total_shares.is_zero() {
            return Err(LiquidStakingError::EmptyPool);
        }
        // wrf = shares * total_underlying / total_shares
        let wrf = shares
            .0
            .checked_mul(self.total_underlying.0)
            .and_then(|v| v.checked_div(self.total_shares.0))
            .ok_or_else(|| LiquidStakingError::Defi(DefiError::Overflow {
                operation: "shares_to_wrf".into(),
            }))?;
        Ok(Amount::new(wrf))
    }

    /// Mint new shares for a WRF deposit. Updates internal totals.
    pub fn mint(&mut self, wrf_amount: Amount) -> LiquidStakingResult<Amount> {
        let shares = self.wrf_to_shares(wrf_amount)?;
        self.total_underlying = self
            .total_underlying
            .checked_add(wrf_amount)
            .ok_or_else(|| LiquidStakingError::Defi(DefiError::Overflow {
                operation: "mint total_underlying".into(),
            }))?;
        self.total_shares = self
            .total_shares
            .checked_add(shares)
            .ok_or_else(|| LiquidStakingError::Defi(DefiError::Overflow {
                operation: "mint total_shares".into(),
            }))?;
        Ok(shares)
    }

    /// Burn shares when a user requests unstaking. Returns WRF owed.
    pub fn burn(&mut self, shares: Amount) -> LiquidStakingResult<Amount> {
        let wrf = self.shares_to_wrf(shares)?;
        self.total_shares = self
            .total_shares
            .checked_sub(shares)
            .ok_or_else(|| LiquidStakingError::Defi(DefiError::InsufficientBalance {
                required: shares,
                available: self.total_shares,
            }))?;
        self.total_underlying = self
            .total_underlying
            .checked_sub(wrf)
            .ok_or_else(|| LiquidStakingError::Defi(DefiError::InsufficientBalance {
                required: wrf,
                available: self.total_underlying,
            }))?;
        Ok(wrf)
    }

    /// Increase `total_underlying` by `reward_amount` without minting new shares.
    /// This increases the WRF value of every outstanding share (auto-compounding).
    pub fn distribute_rewards(&mut self, reward_amount: Amount) -> LiquidStakingResult<()> {
        self.total_underlying = self
            .total_underlying
            .checked_add(reward_amount)
            .ok_or_else(|| LiquidStakingError::Defi(DefiError::Overflow {
                operation: "distribute_rewards".into(),
            }))?;
        Ok(())
    }

    /// Reduce `total_underlying` by `slash_amount` (validator slashing event).
    /// All stWRF holders are diluted proportionally.
    pub fn apply_slash(&mut self, slash_amount: Amount) -> LiquidStakingResult<()> {
        self.total_underlying = self
            .total_underlying
            .checked_sub(slash_amount)
            .ok_or_else(|| LiquidStakingError::Defi(DefiError::InsufficientBalance {
                required: slash_amount,
                available: self.total_underlying,
            }))?;
        Ok(())
    }

    /// Exchange rate as a fraction: `(total_underlying * PRECISION) / total_shares`.
    ///
    /// Returns `(PRECISION, 1)` — i.e. rate = 1.0 — for an empty pool (initial stake).
    pub fn exchange_rate_scaled(&self) -> (u128, u128) {
        if self.total_shares.is_zero() {
            return (PRECISION, PRECISION);
        }
        let scaled = self
            .total_underlying
            .0
            .checked_mul(PRECISION)
            .unwrap_or(u128::MAX);
        (scaled, self.total_shares.0)
    }
}

impl Default for StWRFToken {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// LiquidStakingPool
// ---------------------------------------------------------------------------

/// Core liquid staking pool.
///
/// Tracks per-user share balances, manages the unbonding queue, and delegates
/// maths to `StWRFToken`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiquidStakingPool {
    /// stWRF token state (share ↔ underlying exchange rate).
    pub token: StWRFToken,
    /// Per-user stWRF share balances.
    pub shares: HashMap<Address, Amount>,
    /// Per-user unbonding queues.
    pub unbonding: HashMap<Address, UnbondingQueue>,
    /// Validator allocation table.
    pub validators: Vec<ValidatorAllocation>,
    /// Staking reward rate in basis points per epoch (informational; rewards are
    /// applied via `distribute_rewards`).
    pub reward_rate_bps: u64,
    /// Number of blocks a user must wait before claiming unstaked WRF.
    pub unbonding_delay_blocks: u64,
}

impl LiquidStakingPool {
    /// Create a new pool with the given validator allocations.
    ///
    /// Validator weights must sum to exactly `TOTAL_WEIGHT_BPS`.
    pub fn new(
        validators: Vec<ValidatorAllocation>,
        reward_rate_bps: u64,
        unbonding_delay_blocks: u64,
    ) -> LiquidStakingResult<Self> {
        Self::validate_validators(&validators)?;
        Ok(Self {
            token: StWRFToken::new(),
            shares: HashMap::new(),
            unbonding: HashMap::new(),
            validators,
            reward_rate_bps,
            unbonding_delay_blocks,
        })
    }

    fn validate_validators(validators: &[ValidatorAllocation]) -> LiquidStakingResult<()> {
        let total: u64 = validators.iter().map(|v| v.weight_bps).sum();
        if total != TOTAL_WEIGHT_BPS {
            return Err(LiquidStakingError::InvalidValidatorWeights { actual: total });
        }
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Stake / Unstake
    // -----------------------------------------------------------------------

    /// Deposit `wrf_amount` WRF into the pool. Returns the stWRF shares minted.
    pub fn stake(&mut self, user: Address, wrf_amount: Amount) -> LiquidStakingResult<Amount> {
        if wrf_amount.is_zero() {
            return Err(LiquidStakingError::Defi(DefiError::InvalidAmount {
                reason: "stake amount must be non-zero".into(),
            }));
        }
        let shares_minted = self.token.mint(wrf_amount)?;
        let user_shares = self.shares.entry(user).or_insert(Amount::ZERO);
        *user_shares = user_shares
            .checked_add(shares_minted)
            .ok_or_else(|| LiquidStakingError::Defi(DefiError::Overflow {
                operation: "stake user shares".into(),
            }))?;
        Ok(shares_minted)
    }

    /// Begin unbonding `stwrf_shares` stWRF shares. The corresponding WRF is
    /// locked until `current_height + unbonding_delay_blocks`.
    ///
    /// Returns the WRF amount that will be claimable after the delay.
    pub fn unstake(
        &mut self,
        user: Address,
        stwrf_shares: Amount,
        current_height: u64,
    ) -> LiquidStakingResult<Amount> {
        if stwrf_shares.is_zero() {
            return Err(LiquidStakingError::Defi(DefiError::InvalidAmount {
                reason: "unstake shares must be non-zero".into(),
            }));
        }

        // Deduct from user's share balance.
        let user_shares = self
            .shares
            .get_mut(&user)
            .ok_or_else(|| LiquidStakingError::Defi(DefiError::InsufficientBalance {
                required: stwrf_shares,
                available: Amount::ZERO,
            }))?;

        if *user_shares < stwrf_shares {
            return Err(LiquidStakingError::Defi(DefiError::InsufficientBalance {
                required: stwrf_shares,
                available: *user_shares,
            }));
        }

        *user_shares = user_shares
            .checked_sub(stwrf_shares)
            .ok_or_else(|| LiquidStakingError::Defi(DefiError::Overflow {
                operation: "unstake user shares".into(),
            }))?;

        if user_shares.is_zero() {
            self.shares.remove(&user);
        }

        // Burn shares from the token and get the WRF owed.
        let wrf_owed = self.token.burn(stwrf_shares)?;

        // Enqueue unbonding entry.
        let unlock_height = current_height + self.unbonding_delay_blocks;
        let queue = self.unbonding.entry(user).or_default();
        queue.push(UnbondingEntry {
            amount: wrf_owed,
            unlock_height,
        });

        Ok(wrf_owed)
    }

    /// Claim all unlocked WRF for `user` at the given block height.
    pub fn claim_unbonded(
        &mut self,
        user: Address,
        current_height: u64,
    ) -> LiquidStakingResult<Amount> {
        let queue = self
            .unbonding
            .get_mut(&user)
            .ok_or(LiquidStakingError::NothingToClaim { current_height })?;
        let amount = queue.claim_ready(current_height)?;
        if queue.is_empty() {
            self.unbonding.remove(&user);
        }
        Ok(amount)
    }

    // -----------------------------------------------------------------------
    // Rewards
    // -----------------------------------------------------------------------

    /// Distribute staking rewards. Increases the WRF-per-stWRF exchange rate.
    pub fn distribute_rewards(&mut self, reward_amount: Amount) -> LiquidStakingResult<()> {
        self.token.distribute_rewards(reward_amount)
    }

    // -----------------------------------------------------------------------
    // Slashing
    // -----------------------------------------------------------------------

    /// Apply a slash from a specific validator.
    ///
    /// `slash_amount` is deducted from `total_underlying`, diluting all stWRF
    /// holders proportionally. Validators are identified by their `validator_id`.
    pub fn slash_validator(
        &mut self,
        validator_id: u64,
        slash_amount: Amount,
    ) -> LiquidStakingResult<()> {
        // Verify the validator exists in our allocation table.
        if !self.validators.iter().any(|v| v.validator_id == validator_id) {
            return Err(LiquidStakingError::ValidatorNotFound { validator_id });
        }
        self.token.apply_slash(slash_amount)
    }

    // -----------------------------------------------------------------------
    // Validator management
    // -----------------------------------------------------------------------

    /// Replace the validator allocation table.
    ///
    /// New allocations must still sum to `TOTAL_WEIGHT_BPS`.
    pub fn rebalance_validators(
        &mut self,
        new_allocations: Vec<ValidatorAllocation>,
    ) -> LiquidStakingResult<()> {
        Self::validate_validators(&new_allocations)?;
        self.validators = new_allocations;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Queries
    // -----------------------------------------------------------------------

    /// Return the stWRF share balance of `user`.
    pub fn share_balance(&self, user: &Address) -> Amount {
        self.shares.get(user).copied().unwrap_or(Amount::ZERO)
    }

    /// Return the WRF value of `user`'s stWRF shares at the current exchange rate.
    pub fn underlying_balance(&self, user: &Address) -> LiquidStakingResult<Amount> {
        let shares = self.share_balance(user);
        self.token.shares_to_wrf(shares)
    }

    /// Collateral value of `stwrf_shares` after applying the 75% LTV.
    ///
    /// Used by `lending::CollateralManager` when computing health factors.
    pub fn collateral_value_bps(&self, stwrf_shares: Amount) -> LiquidStakingResult<Amount> {
        let wrf_value = self.token.shares_to_wrf(stwrf_shares)?;
        wrf_value
            .mul_fraction(STWRF_LTV_BPS as u128, 10_000)
            .ok_or_else(|| LiquidStakingError::Defi(DefiError::Overflow {
                operation: "collateral_value_bps".into(),
            }))
    }

    /// Total WRF staked in the pool (includes accrued rewards, minus slashes).
    pub fn total_staked(&self) -> Amount {
        self.token.total_underlying
    }

    /// Total stWRF shares outstanding.
    pub fn total_shares(&self) -> Amount {
        self.token.total_shares
    }
}

// ---------------------------------------------------------------------------
// stWRF AssetId helper
// ---------------------------------------------------------------------------

/// Returns the canonical `AssetId` for stWRF.
pub fn stwrf_asset_id() -> AssetId {
    AssetId::new(STWRF_ASSET_ID)
}

// ---------------------------------------------------------------------------
// Collateral integration helper
// ---------------------------------------------------------------------------

/// Returns a `CollateralConfig` for stWRF with 75% LTV and 80% liquidation threshold,
/// ready to be passed to `lending::CollateralManager::add_collateral`.
///
/// Import `wrfcoin_lending::collateral::CollateralConfig` at the call site.
pub fn stwrf_collateral_config_params() -> (u64, u64) {
    (STWRF_LTV_BPS, STWRF_LIQUIDATION_THRESHOLD_BPS)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(n: u8) -> Address {
        let mut bytes = [0u8; 32];
        bytes[31] = n;
        Address::from_bytes(bytes)
    }

    fn single_validator_pool() -> LiquidStakingPool {
        LiquidStakingPool::new(
            vec![ValidatorAllocation::new(1, 10_000)],
            500,  // 5% reward rate bps
            100,  // 100 block unbonding delay
        )
        .unwrap()
    }

    fn multi_validator_pool() -> LiquidStakingPool {
        LiquidStakingPool::new(
            vec![
                ValidatorAllocation::new(1, 6_000), // 60%
                ValidatorAllocation::new(2, 3_000), // 30%
                ValidatorAllocation::new(3, 1_000), // 10%
            ],
            500,
            100,
        )
        .unwrap()
    }

    // -----------------------------------------------------------------------
    // Exchange rate tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_exchange_rate_increases_after_rewards() {
        // 100 WRF staked → 100 shares at 1:1
        let mut pool = single_validator_pool();
        pool.stake(addr(1), Amount::new(100)).unwrap();

        assert_eq!(pool.total_staked(), Amount::new(100));
        assert_eq!(pool.total_shares(), Amount::new(100));

        // Distribute 10 WRF reward → 110 WRF underlying, 100 shares
        pool.distribute_rewards(Amount::new(10)).unwrap();

        assert_eq!(pool.total_staked(), Amount::new(110));
        assert_eq!(pool.total_shares(), Amount::new(100));

        // Each share is now worth 1.1 WRF
        let wrf = pool.token.shares_to_wrf(Amount::new(100)).unwrap();
        assert_eq!(wrf, Amount::new(110));

        // Exchange rate scaled: num = 110 * PRECISION, den = 100 (total_shares).
        // num / den = 110 * PRECISION / 100 = 1.1 * PRECISION.
        let (num, den) = pool.token.exchange_rate_scaled();
        let rate_scaled = num / den; // = 1.1 * PRECISION
        // 1.1 * 1e18 should be between 1e18 and 2e18.
        assert!(rate_scaled > PRECISION, "rate should be > 1.0 after rewards");
        assert!(rate_scaled < 2 * PRECISION, "rate should be < 2.0");
        // Exact: 110 * PRECISION / 100 = 11 * PRECISION / 10
        assert_eq!(rate_scaled, 11 * PRECISION / 10);
    }

    #[test]
    fn test_second_stake_uses_updated_rate() {
        let mut pool = single_validator_pool();
        pool.stake(addr(1), Amount::new(100)).unwrap();
        pool.distribute_rewards(Amount::new(10)).unwrap();

        // Now rate = 110/100 = 1.1 WRF per share.
        // A second user deposits 110 WRF → should get 100 shares.
        let shares = pool.stake(addr(2), Amount::new(110)).unwrap();
        assert_eq!(shares, Amount::new(100));
    }

    // -----------------------------------------------------------------------
    // Slash propagation tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_slash_reduces_all_holders_proportionally() {
        let mut pool = multi_validator_pool();

        // User 1 stakes 200 WRF → 200 shares
        pool.stake(addr(1), Amount::new(200)).unwrap();
        // User 2 stakes 800 WRF → 800 shares
        pool.stake(addr(2), Amount::new(800)).unwrap();

        // Total: 1000 WRF, 1000 shares. Rate = 1.0.
        assert_eq!(pool.total_staked().0, 1000);
        assert_eq!(pool.total_shares().0, 1000);

        // Validator 1 (60% weight) is slashed 100 WRF.
        pool.slash_validator(1, Amount::new(100)).unwrap();

        // total_staked drops to 900; shares stay at 1000.
        assert_eq!(pool.total_staked().0, 900);
        assert_eq!(pool.total_shares().0, 1000);

        // User 1 held 200 shares → now worth 200 * 900 / 1000 = 180 WRF.
        let u1_wrf = pool.underlying_balance(&addr(1)).unwrap();
        assert_eq!(u1_wrf, Amount::new(180));

        // User 2 held 800 shares → now worth 800 * 900 / 1000 = 720 WRF.
        let u2_wrf = pool.underlying_balance(&addr(2)).unwrap();
        assert_eq!(u2_wrf, Amount::new(720));

        // Combined = 900.
        assert_eq!(u1_wrf.0 + u2_wrf.0, 900);
    }

    #[test]
    fn test_slash_unknown_validator_fails() {
        let mut pool = single_validator_pool();
        pool.stake(addr(1), Amount::new(100)).unwrap();
        let result = pool.slash_validator(99, Amount::new(10));
        assert!(matches!(result, Err(LiquidStakingError::ValidatorNotFound { .. })));
    }

    // -----------------------------------------------------------------------
    // Unbonding / unstaking tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_unstake_with_unbonding_delay() {
        let mut pool = single_validator_pool(); // 100-block delay
        pool.stake(addr(1), Amount::new(100)).unwrap();

        // Unstake all shares at height 500.
        let wrf_locked = pool.unstake(addr(1), Amount::new(100), 500).unwrap();
        assert_eq!(wrf_locked, Amount::new(100));

        // Cannot claim before height 600.
        let result = pool.claim_unbonded(addr(1), 599);
        assert!(matches!(result, Err(LiquidStakingError::NothingToClaim { .. })));

        // Can claim at height 600.
        let claimed = pool.claim_unbonded(addr(1), 600).unwrap();
        assert_eq!(claimed, Amount::new(100));
    }

    #[test]
    fn test_unstake_partial_shares() {
        let mut pool = single_validator_pool();
        pool.stake(addr(1), Amount::new(200)).unwrap();

        // Unstake half.
        pool.unstake(addr(1), Amount::new(100), 0).unwrap();
        assert_eq!(pool.share_balance(&addr(1)), Amount::new(100));
        assert_eq!(pool.total_shares().0, 100);
        assert_eq!(pool.total_staked().0, 100);
    }

    #[test]
    fn test_unstake_more_than_balance_fails() {
        let mut pool = single_validator_pool();
        pool.stake(addr(1), Amount::new(100)).unwrap();
        let result = pool.unstake(addr(1), Amount::new(200), 0);
        assert!(matches!(
            result,
            Err(LiquidStakingError::Defi(DefiError::InsufficientBalance { .. }))
        ));
    }

    #[test]
    fn test_unbonding_queue_multiple_entries() {
        let mut pool = single_validator_pool(); // 100-block delay
        pool.stake(addr(1), Amount::new(300)).unwrap();

        // Unstake at height 100 → unlock at 200.
        pool.unstake(addr(1), Amount::new(100), 100).unwrap();
        // Unstake at height 150 → unlock at 250.
        pool.unstake(addr(1), Amount::new(100), 150).unwrap();
        // Unstake at height 200 → unlock at 300.
        pool.unstake(addr(1), Amount::new(100), 200).unwrap();

        // At height 249: only first entry is claimable.
        let claimed = pool.claim_unbonded(addr(1), 249).unwrap();
        assert_eq!(claimed.0, 100);

        // At height 300: second and third entries claimable.
        let claimed2 = pool.claim_unbonded(addr(1), 300).unwrap();
        assert_eq!(claimed2.0, 200);
    }

    // -----------------------------------------------------------------------
    // Collateral valuation tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_collateral_value_of_stwrf() {
        let mut pool = single_validator_pool();
        // Stake 1000 WRF → 1000 shares at rate 1.0.
        pool.stake(addr(1), Amount::new(1000)).unwrap();

        // Distribute 100 WRF rewards → rate = 1100/1000 = 1.1 WRF/share.
        pool.distribute_rewards(Amount::new(100)).unwrap();

        // User holds 1000 shares → WRF value = 1100.
        // Collateral value at 75% LTV = 1100 * 7500 / 10000 = 825.
        let col_val = pool.collateral_value_bps(Amount::new(1000)).unwrap();
        assert_eq!(col_val, Amount::new(825));
    }

    #[test]
    fn test_collateral_value_constant_rate() {
        let mut pool = single_validator_pool();
        pool.stake(addr(1), Amount::new(1000)).unwrap();
        // No rewards; rate = 1.0. Collateral = 1000 * 0.75 = 750.
        let col_val = pool.collateral_value_bps(Amount::new(1000)).unwrap();
        assert_eq!(col_val, Amount::new(750));
    }

    #[test]
    fn test_stwrf_collateral_config_params() {
        let (ltv, threshold) = stwrf_collateral_config_params();
        assert_eq!(ltv, 7_500);
        assert_eq!(threshold, 8_000);
        assert!(threshold >= ltv, "liquidation threshold must be >= LTV");
    }

    // -----------------------------------------------------------------------
    // Multi-validator tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_multi_validator_weighted_allocation() {
        let pool = multi_validator_pool();
        let total_weight: u64 = pool.validators.iter().map(|v| v.weight_bps).sum();
        assert_eq!(total_weight, TOTAL_WEIGHT_BPS);
        assert_eq!(pool.validators[0].weight_bps, 6_000);
        assert_eq!(pool.validators[1].weight_bps, 3_000);
        assert_eq!(pool.validators[2].weight_bps, 1_000);
    }

    #[test]
    fn test_validator_weights_must_sum_to_10000() {
        let result = LiquidStakingPool::new(
            vec![
                ValidatorAllocation::new(1, 5_000),
                ValidatorAllocation::new(2, 4_000), // only 9000 total
            ],
            500,
            100,
        );
        assert!(matches!(
            result,
            Err(LiquidStakingError::InvalidValidatorWeights { .. })
        ));
    }

    #[test]
    fn test_rebalance_validators() {
        let mut pool = multi_validator_pool();
        pool.rebalance_validators(vec![
            ValidatorAllocation::new(1, 5_000),
            ValidatorAllocation::new(2, 5_000),
        ])
        .unwrap();
        assert_eq!(pool.validators.len(), 2);
        assert_eq!(pool.validators[0].weight_bps, 5_000);
    }

    #[test]
    fn test_rebalance_invalid_weights_fails() {
        let mut pool = multi_validator_pool();
        let result = pool.rebalance_validators(vec![
            ValidatorAllocation::new(1, 3_000),
        ]);
        assert!(matches!(
            result,
            Err(LiquidStakingError::InvalidValidatorWeights { .. })
        ));
    }

    // -----------------------------------------------------------------------
    // Edge / misc tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_stake_zero_fails() {
        let mut pool = single_validator_pool();
        let result = pool.stake(addr(1), Amount::ZERO);
        assert!(result.is_err());
    }

    #[test]
    fn test_stwrf_asset_id() {
        let id = stwrf_asset_id();
        assert_eq!(id.as_str(), "stWRF");
    }

    #[test]
    fn test_initial_exchange_rate_is_one_to_one() {
        let _pool = single_validator_pool();
        // On empty pool, shares_to_wrf should return the same amount.
        // But empty pool has zero shares so we test after first stake.
        let mut p = single_validator_pool();
        p.stake(addr(1), Amount::new(500)).unwrap();
        let wrf = p.token.shares_to_wrf(Amount::new(500)).unwrap();
        assert_eq!(wrf, Amount::new(500));
    }

    #[test]
    fn test_empty_pool_shares_to_wrf_zero() {
        let pool = single_validator_pool();
        let result = pool.token.shares_to_wrf(Amount::ZERO).unwrap();
        assert_eq!(result, Amount::ZERO);
    }
}
