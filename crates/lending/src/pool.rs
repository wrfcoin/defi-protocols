use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use wrfcoin_defi_common::error::{DefiError, DefiResult};
use wrfcoin_defi_common::types::{Amount, AssetId, Timestamp};

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
