use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use wrfcoin_defi_common::error::{DefiError, DefiResult};
use wrfcoin_defi_common::types::{Amount, AssetId, Timestamp};

use crate::interest::InterestRateModel;

/// Seconds in a year (365.25 days) for annualized rate calculations.
const SECONDS_PER_YEAR: u128 = 365 * 24 * 3600;

/// Data tracked per reserve (asset) in the lending pool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReserveData {
    /// Total amount deposited by suppliers.
    pub total_deposits: Amount,
    /// Total amount currently borrowed.
    pub total_borrows: Amount,
    /// Current annualized interest rate in basis points (e.g., 500 = 5%).
    pub interest_rate_bps: u64,
    /// Timestamp of the last accrual update.
    pub last_update: Timestamp,
}

impl ReserveData {
    pub fn new(timestamp: Timestamp) -> Self {
        Self {
            total_deposits: Amount::ZERO,
            total_borrows: Amount::ZERO,
            interest_rate_bps: 0,
            last_update: timestamp,
        }
    }

    /// Utilization rate as basis points (0..10_000).
    pub fn utilization_bps(&self) -> u64 {
        if self.total_deposits.is_zero() {
            return 0;
        }
        // (borrows * 10_000) / deposits
        let num = (self.total_borrows.0 as u128)
            .checked_mul(10_000)
            .unwrap_or(u128::MAX);
        (num / self.total_deposits.0) as u64
    }
}

/// Manages reserves for a lending protocol.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LendingPool {
    /// Reserve state keyed by asset.
    pub reserves: HashMap<AssetId, ReserveData>,
}

impl LendingPool {
    pub fn new() -> Self {
        Self {
            reserves: HashMap::new(),
        }
    }

    /// Initialize a reserve for a new asset. No-op if already present.
    pub fn init_reserve(&mut self, asset: AssetId, timestamp: Timestamp) {
        self.reserves
            .entry(asset)
            .or_insert_with(|| ReserveData::new(timestamp));
    }

    /// Deposit tokens into a reserve, increasing total deposits.
    pub fn deposit(
        &mut self,
        asset: &AssetId,
        amount: Amount,
        timestamp: Timestamp,
    ) -> DefiResult<()> {
        if amount.is_zero() {
            return Err(DefiError::InvalidAmount {
                reason: "deposit amount must be non-zero".into(),
            });
        }

        let reserve = self.reserves.get_mut(asset).ok_or_else(|| DefiError::InvalidAsset {
            asset_id: asset.clone(),
        })?;

        reserve.total_deposits = reserve
            .total_deposits
            .checked_add(amount)
            .ok_or_else(|| DefiError::Overflow {
                operation: "deposit".into(),
            })?;
        reserve.last_update = timestamp;

        Ok(())
    }

    /// Withdraw tokens from a reserve, decreasing total deposits.
    pub fn withdraw(
        &mut self,
        asset: &AssetId,
        amount: Amount,
        timestamp: Timestamp,
    ) -> DefiResult<()> {
        if amount.is_zero() {
            return Err(DefiError::InvalidAmount {
                reason: "withdraw amount must be non-zero".into(),
            });
        }

        let reserve = self.reserves.get_mut(asset).ok_or_else(|| DefiError::InvalidAsset {
            asset_id: asset.clone(),
        })?;

        let available = available_liquidity_of(reserve);
        if amount > available {
            return Err(DefiError::InsufficientLiquidity {
                requested: amount,
                available,
            });
        }

        reserve.total_deposits = reserve
            .total_deposits
            .checked_sub(amount)
            .ok_or_else(|| DefiError::Overflow {
                operation: "withdraw".into(),
            })?;
        reserve.last_update = timestamp;

        Ok(())
    }

    /// Borrow tokens from a reserve. Increases total_borrows.
    pub fn borrow(
        &mut self,
        asset: &AssetId,
        amount: Amount,
        timestamp: Timestamp,
    ) -> DefiResult<()> {
        if amount.is_zero() {
            return Err(DefiError::InvalidAmount {
                reason: "borrow amount must be non-zero".into(),
            });
        }

        let reserve = self.reserves.get_mut(asset).ok_or_else(|| DefiError::InvalidAsset {
            asset_id: asset.clone(),
        })?;

        let available = available_liquidity_of(reserve);
        if amount > available {
            return Err(DefiError::InsufficientLiquidity {
                requested: amount,
                available,
            });
        }

        reserve.total_borrows = reserve
            .total_borrows
            .checked_add(amount)
            .ok_or_else(|| DefiError::Overflow {
                operation: "borrow".into(),
            })?;
        reserve.last_update = timestamp;

        Ok(())
    }

    /// Repay borrowed tokens to a reserve. Decreases total_borrows.
    /// If `amount` exceeds outstanding borrows, only the outstanding amount is repaid.
    /// Returns the actual amount repaid.
    pub fn repay(
        &mut self,
        asset: &AssetId,
        amount: Amount,
        timestamp: Timestamp,
    ) -> DefiResult<Amount> {
        if amount.is_zero() {
            return Err(DefiError::InvalidAmount {
                reason: "repay amount must be non-zero".into(),
            });
        }

        let reserve = self.reserves.get_mut(asset).ok_or_else(|| DefiError::InvalidAsset {
            asset_id: asset.clone(),
        })?;

        // Cap repayment at outstanding borrows.
        let repay_amount = if amount > reserve.total_borrows {
            reserve.total_borrows
        } else {
            amount
        };

        if repay_amount.is_zero() {
            return Err(DefiError::InvalidAmount {
                reason: "no outstanding borrows to repay".into(),
            });
        }

        reserve.total_borrows = reserve
            .total_borrows
            .checked_sub(repay_amount)
            .ok_or_else(|| DefiError::Overflow {
                operation: "repay".into(),
            })?;
        reserve.last_update = timestamp;

        Ok(repay_amount)
    }

    /// Accrue interest on a reserve based on elapsed time and the interest rate model.
    ///
    /// This should be called before any deposit/withdraw/borrow/repay to ensure
    /// interest is up-to-date.
    pub fn accrue_interest(
        &mut self,
        asset: &AssetId,
        model: &dyn InterestRateModel,
        timestamp: Timestamp,
    ) -> DefiResult<()> {
        let reserve = self.reserves.get_mut(asset).ok_or_else(|| DefiError::InvalidAsset {
            asset_id: asset.clone(),
        })?;

        if timestamp <= reserve.last_update {
            return Ok(());
        }

        if reserve.total_borrows.is_zero() {
            reserve.last_update = timestamp;
            return Ok(());
        }

        let elapsed = timestamp.elapsed_since(reserve.last_update) as u128;
        let utilization = reserve.utilization_bps();
        let borrow_rate_bps = model.calculate_borrow_rate(utilization) as u128;

        // interest = total_borrows * borrow_rate_bps * elapsed / (SECONDS_PER_YEAR * 10_000)
        let interest = reserve
            .total_borrows
            .0
            .checked_mul(borrow_rate_bps)
            .and_then(|v| v.checked_mul(elapsed))
            .and_then(|v| v.checked_div(SECONDS_PER_YEAR * 10_000))
            .ok_or(DefiError::Overflow {
                operation: "accrue_interest".into(),
            })?;

        if interest > 0 {
            reserve.total_borrows = reserve
                .total_borrows
                .checked_add(Amount::new(interest))
                .ok_or(DefiError::Overflow {
                    operation: "accrue_interest borrows".into(),
                })?;
        }

        // Update stored rate and timestamp.
        reserve.interest_rate_bps = model.calculate_borrow_rate(reserve.utilization_bps());
        reserve.last_update = timestamp;

        Ok(())
    }

    /// Available liquidity for a given asset (deposits minus borrows).
    pub fn available_liquidity(&self, asset: &AssetId) -> DefiResult<Amount> {
        let reserve = self.reserves.get(asset).ok_or_else(|| DefiError::InvalidAsset {
            asset_id: asset.clone(),
        })?;
        Ok(available_liquidity_of(reserve))
    }
}

/// Compute available liquidity from a reserve without borrowing the pool.
fn available_liquidity_of(reserve: &ReserveData) -> Amount {
    Amount(reserve.total_deposits.0.saturating_sub(reserve.total_borrows.0))
}

impl Default for LendingPool {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(secs: u64) -> Timestamp {
        Timestamp::new(secs)
    }

    fn asset(name: &str) -> AssetId {
        AssetId::new(name)
    }

    #[test]
    fn test_reserve_data_new() {
        let r = ReserveData::new(ts(100));
        assert!(r.total_deposits.is_zero());
        assert!(r.total_borrows.is_zero());
        assert_eq!(r.utilization_bps(), 0);
    }

    #[test]
    fn test_utilization_bps() {
        let mut r = ReserveData::new(ts(0));
        r.total_deposits = Amount::new(10_000);
        r.total_borrows = Amount::new(8_000);
        assert_eq!(r.utilization_bps(), 8_000); // 80%
    }

    #[test]
    fn test_utilization_zero_deposits() {
        let r = ReserveData::new(ts(0));
        assert_eq!(r.utilization_bps(), 0);
    }

    #[test]
    fn test_init_reserve() {
        let mut pool = LendingPool::new();
        pool.init_reserve(asset("WRF"), ts(0));
        assert!(pool.reserves.contains_key(&asset("WRF")));
        // Idempotent
        pool.init_reserve(asset("WRF"), ts(100));
        assert_eq!(pool.reserves.len(), 1);
    }

    #[test]
    fn test_deposit() {
        let mut pool = LendingPool::new();
        pool.init_reserve(asset("WRF"), ts(0));
        pool.deposit(&asset("WRF"), Amount::new(1000), ts(1)).expect("deposit should succeed");
        assert_eq!(pool.reserves[&asset("WRF")].total_deposits, Amount::new(1000));
    }

    #[test]
    fn test_deposit_zero_fails() {
        let mut pool = LendingPool::new();
        pool.init_reserve(asset("WRF"), ts(0));
        let result = pool.deposit(&asset("WRF"), Amount::ZERO, ts(1));
        assert!(result.is_err());
    }

    #[test]
    fn test_deposit_unknown_asset_fails() {
        let mut pool = LendingPool::new();
        let result = pool.deposit(&asset("WRF"), Amount::new(100), ts(1));
        assert!(result.is_err());
    }

    #[test]
    fn test_withdraw() {
        let mut pool = LendingPool::new();
        pool.init_reserve(asset("WRF"), ts(0));
        pool.deposit(&asset("WRF"), Amount::new(1000), ts(1)).expect("deposit should succeed");
        pool.withdraw(&asset("WRF"), Amount::new(500), ts(2)).expect("withdraw should succeed");
        assert_eq!(pool.reserves[&asset("WRF")].total_deposits, Amount::new(500));
    }

    #[test]
    fn test_withdraw_exceeds_available() {
        let mut pool = LendingPool::new();
        pool.init_reserve(asset("WRF"), ts(0));
        pool.deposit(&asset("WRF"), Amount::new(100), ts(1)).expect("deposit should succeed");
        let result = pool.withdraw(&asset("WRF"), Amount::new(200), ts(2));
        assert!(result.is_err());
    }

    #[test]
    fn test_withdraw_zero_fails() {
        let mut pool = LendingPool::new();
        pool.init_reserve(asset("WRF"), ts(0));
        pool.deposit(&asset("WRF"), Amount::new(100), ts(1)).expect("deposit should succeed");
        let result = pool.withdraw(&asset("WRF"), Amount::ZERO, ts(2));
        assert!(result.is_err());
    }

    #[test]
    fn test_available_liquidity() {
        let mut pool = LendingPool::new();
        pool.init_reserve(asset("WRF"), ts(0));
        pool.deposit(&asset("WRF"), Amount::new(1000), ts(1)).expect("deposit should succeed");
        // Simulate borrow by modifying total_borrows directly
        pool.reserves.get_mut(&asset("WRF")).expect("WRF reserve should exist").total_borrows = Amount::new(300);
        assert_eq!(pool.available_liquidity(&asset("WRF")).expect("available_liquidity should succeed"), Amount::new(700));
    }
}
