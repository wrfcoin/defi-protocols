use serde::{Deserialize, Serialize};
use wrfcoin_defi_common::error::{DefiError, DefiResult};
use wrfcoin_defi_common::types::{Amount, AssetId, PoolId};

/// A constant-product (x * y = k) automated market maker liquidity pool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiquidityPool {
    /// Unique pool identifier.
    pub pool_id: PoolId,
    /// Token A identifier.
    pub asset_a: AssetId,
    /// Token B identifier.
    pub asset_b: AssetId,
    /// Current reserve of token A.
    pub reserve_a: Amount,
    /// Current reserve of token B.
    pub reserve_b: Amount,
    /// Total supply of LP tokens.
    pub total_lp_supply: Amount,
    /// Swap fee rate in basis points (e.g., 30 = 0.3%).
    pub fee_rate_bps: u64,
}

impl LiquidityPool {
    pub fn new(
        pool_id: PoolId,
        asset_a: AssetId,
        asset_b: AssetId,
        fee_rate_bps: u64,
    ) -> Self {
        Self {
            pool_id,
            asset_a,
            asset_b,
            reserve_a: Amount::ZERO,
            reserve_b: Amount::ZERO,
            total_lp_supply: Amount::ZERO,
            fee_rate_bps,
        }
    }

    /// Add liquidity to the pool. Returns the number of LP tokens minted.
    ///
    /// On first deposit, LP tokens = sqrt(amount_a * amount_b).
    /// On subsequent deposits, LP tokens are proportional to the smaller ratio.
    pub fn add_liquidity(
        &mut self,
        amount_a: Amount,
        amount_b: Amount,
    ) -> DefiResult<Amount> {
        if amount_a.is_zero() || amount_b.is_zero() {
            return Err(DefiError::InvalidAmount {
                reason: "both amounts must be non-zero".into(),
            });
        }

        let lp_minted = if self.total_lp_supply.is_zero() {
            // Initial liquidity: LP tokens = sqrt(a * b)
            let product = amount_a
                .0
                .checked_mul(amount_b.0)
                .ok_or(DefiError::Overflow {
                    operation: "initial liquidity product".into(),
                })?;
            Amount(integer_sqrt(product))
        } else {
            // Proportional deposit: min(amount_a * total_lp / reserve_a, amount_b * total_lp / reserve_b)
            let lp_from_a = amount_a
                .0
                .checked_mul(self.total_lp_supply.0)
                .and_then(|v| v.checked_div(self.reserve_a.0))
                .ok_or(DefiError::Overflow {
                    operation: "add_liquidity lp_from_a".into(),
                })?;

            let lp_from_b = amount_b
                .0
                .checked_mul(self.total_lp_supply.0)
                .and_then(|v| v.checked_div(self.reserve_b.0))
                .ok_or(DefiError::Overflow {
                    operation: "add_liquidity lp_from_b".into(),
                })?;

            Amount(lp_from_a.min(lp_from_b))
        };

        if lp_minted.is_zero() {
            return Err(DefiError::InvalidAmount {
                reason: "minted LP amount would be zero".into(),
            });
        }

        self.reserve_a = self.reserve_a.checked_add(amount_a).ok_or(DefiError::Overflow {
            operation: "add_liquidity reserve_a".into(),
        })?;
        self.reserve_b = self.reserve_b.checked_add(amount_b).ok_or(DefiError::Overflow {
            operation: "add_liquidity reserve_b".into(),
        })?;
        self.total_lp_supply = self
            .total_lp_supply
            .checked_add(lp_minted)
            .ok_or(DefiError::Overflow {
                operation: "add_liquidity lp_supply".into(),
            })?;

        Ok(lp_minted)
    }

    /// Remove liquidity by burning LP tokens. Returns (amount_a, amount_b).
    pub fn remove_liquidity(
        &mut self,
        lp_amount: Amount,
    ) -> DefiResult<(Amount, Amount)> {
        if lp_amount.is_zero() {
            return Err(DefiError::InvalidAmount {
                reason: "LP burn amount must be non-zero".into(),
            });
        }

        if lp_amount > self.total_lp_supply {
            return Err(DefiError::InsufficientBalance {
                required: lp_amount,
                available: self.total_lp_supply,
            });
        }

        // Proportional withdrawal.
        let amount_a = lp_amount
            .0
            .checked_mul(self.reserve_a.0)
            .and_then(|v| v.checked_div(self.total_lp_supply.0))
            .map(Amount)
            .ok_or(DefiError::Overflow {
                operation: "remove_liquidity amount_a".into(),
            })?;

        let amount_b = lp_amount
            .0
            .checked_mul(self.reserve_b.0)
            .and_then(|v| v.checked_div(self.total_lp_supply.0))
            .map(Amount)
            .ok_or(DefiError::Overflow {
                operation: "remove_liquidity amount_b".into(),
            })?;

        self.reserve_a = self.reserve_a.checked_sub(amount_a).ok_or(DefiError::Overflow {
            operation: "remove_liquidity reserve_a".into(),
        })?;
        self.reserve_b = self.reserve_b.checked_sub(amount_b).ok_or(DefiError::Overflow {
            operation: "remove_liquidity reserve_b".into(),
        })?;
        self.total_lp_supply = self
            .total_lp_supply
            .checked_sub(lp_amount)
            .ok_or(DefiError::Overflow {
                operation: "remove_liquidity lp_supply".into(),
            })?;

        Ok((amount_a, amount_b))
    }

    /// Get the current reserves.
    pub fn get_reserves(&self) -> (Amount, Amount) {
        (self.reserve_a, self.reserve_b)
    }

    /// Compute the constant product invariant k = reserve_a * reserve_b.
    pub fn invariant(&self) -> Option<u128> {
        self.reserve_a.0.checked_mul(self.reserve_b.0)
    }
}

/// Integer square root using Newton's method.
fn integer_sqrt(n: u128) -> u128 {
    if n == 0 {
        return 0;
    }
    let mut x = n;
    let mut y = (x + 1) / 2;
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x
}
