use serde::{Deserialize, Serialize};
use wrfcoin_defi_common::error::{DefiError, DefiResult};
use wrfcoin_defi_common::types::Amount;

use crate::pool::LiquidityPool;

/// Engine for executing swaps against a liquidity pool using the constant product formula.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwapEngine;

impl SwapEngine {
    pub fn new() -> Self {
        Self
    }

    /// Calculate the output amount for a given input amount using the constant product formula.
    ///
    /// Formula: amount_out = (reserve_out * amount_in_after_fee) / (reserve_in + amount_in_after_fee)
    ///
    /// This maintains the invariant: (reserve_in + amount_in_after_fee) * (reserve_out - amount_out) >= k
    pub fn get_amount_out(
        &self,
        amount_in: Amount,
        reserve_in: Amount,
        reserve_out: Amount,
        fee_rate_bps: u64,
    ) -> DefiResult<Amount> {
        if amount_in.is_zero() {
            return Err(DefiError::InvalidAmount {
                reason: "input amount must be non-zero".into(),
            });
        }
        if reserve_in.is_zero() || reserve_out.is_zero() {
            return Err(DefiError::InsufficientLiquidity {
                requested: amount_in,
                available: Amount::ZERO,
            });
        }

        // amount_in_after_fee = amount_in * (10_000 - fee_rate) / 10_000
        let fee_complement = 10_000u128 - fee_rate_bps as u128;
        let amount_in_with_fee = amount_in
            .0
            .checked_mul(fee_complement)
            .ok_or(DefiError::Overflow {
                operation: "swap fee calculation".into(),
            })?;

        // numerator = amount_in_with_fee * reserve_out
        let numerator = amount_in_with_fee
            .checked_mul(reserve_out.0)
            .ok_or(DefiError::Overflow {
                operation: "swap numerator".into(),
            })?;

        // denominator = reserve_in * 10_000 + amount_in_with_fee
        let denominator = reserve_in
            .0
            .checked_mul(10_000)
            .and_then(|v| v.checked_add(amount_in_with_fee))
            .ok_or(DefiError::Overflow {
                operation: "swap denominator".into(),
            })?;

        if denominator == 0 {
            return Err(DefiError::Overflow {
                operation: "swap division by zero".into(),
            });
        }

        Ok(Amount(numerator / denominator))
    }

    /// Calculate the required input amount to receive a specific output amount (reverse calculation).
    ///
    /// Formula: amount_in = (reserve_in * amount_out * 10_000) / ((reserve_out - amount_out) * fee_complement) + 1
    pub fn get_amount_in(
        &self,
        amount_out: Amount,
        reserve_in: Amount,
        reserve_out: Amount,
        fee_rate_bps: u64,
    ) -> DefiResult<Amount> {
        if amount_out.is_zero() {
            return Err(DefiError::InvalidAmount {
                reason: "output amount must be non-zero".into(),
            });
        }
        if amount_out >= reserve_out {
            return Err(DefiError::InsufficientLiquidity {
                requested: amount_out,
                available: reserve_out,
            });
        }
        if reserve_in.is_zero() {
            return Err(DefiError::InsufficientLiquidity {
                requested: amount_out,
                available: Amount::ZERO,
            });
        }

        let fee_complement = 10_000u128 - fee_rate_bps as u128;

        // numerator = reserve_in * amount_out * 10_000
        let numerator = reserve_in
            .0
            .checked_mul(amount_out.0)
            .and_then(|v| v.checked_mul(10_000))
            .ok_or(DefiError::Overflow {
                operation: "get_amount_in numerator".into(),
            })?;

        // denominator = (reserve_out - amount_out) * fee_complement
        let denominator = reserve_out
            .0
            .checked_sub(amount_out.0)
            .and_then(|v| v.checked_mul(fee_complement))
            .ok_or(DefiError::Overflow {
                operation: "get_amount_in denominator".into(),
            })?;

        if denominator == 0 {
            return Err(DefiError::Overflow {
                operation: "get_amount_in division by zero".into(),
            });
        }

        // Round up: (numerator / denominator) + 1
        Ok(Amount(numerator / denominator + 1))
    }

    /// Execute a swap on a liquidity pool, modifying its reserves.
    ///
    /// Returns the output amount. Fails if output is below `min_amount_out` (slippage protection).
    pub fn execute_swap(
        &self,
        pool: &mut LiquidityPool,
        amount_in: Amount,
        is_a_to_b: bool,
        min_amount_out: Amount,
    ) -> DefiResult<Amount> {
        let (reserve_in, reserve_out) = if is_a_to_b {
            (pool.reserve_a, pool.reserve_b)
        } else {
            (pool.reserve_b, pool.reserve_a)
        };

        let amount_out = self.get_amount_out(amount_in, reserve_in, reserve_out, pool.fee_rate_bps)?;

        if amount_out < min_amount_out {
            return Err(DefiError::Slippage {
                minimum_out: min_amount_out,
                actual_out: amount_out,
            });
        }

        // Update reserves.
        if is_a_to_b {
            pool.reserve_a = pool.reserve_a.checked_add(amount_in).ok_or(DefiError::Overflow {
                operation: "swap reserve_a increase".into(),
            })?;
            pool.reserve_b = pool.reserve_b.checked_sub(amount_out).ok_or(DefiError::Overflow {
                operation: "swap reserve_b decrease".into(),
            })?;
        } else {
            pool.reserve_b = pool.reserve_b.checked_add(amount_in).ok_or(DefiError::Overflow {
                operation: "swap reserve_b increase".into(),
            })?;
            pool.reserve_a = pool.reserve_a.checked_sub(amount_out).ok_or(DefiError::Overflow {
                operation: "swap reserve_a decrease".into(),
            })?;
        }

        Ok(amount_out)
    }
}

impl Default for SwapEngine {
    fn default() -> Self {
        Self::new()
    }
}
