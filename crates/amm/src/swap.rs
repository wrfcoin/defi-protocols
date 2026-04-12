use serde::{Deserialize, Serialize};
use wrfcoin_defi_common::error::{DefiError, DefiResult};
use wrfcoin_defi_common::types::{Amount, AssetId, Price, Timestamp};
use wrfcoin_twap_oracle::TwapOracle;

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
        let amount_in_with_fee =
            amount_in
                .0
                .checked_mul(fee_complement)
                .ok_or(DefiError::Overflow {
                    operation: "swap fee calculation".into(),
                })?;

        // numerator = amount_in_with_fee * reserve_out
        let numerator =
            amount_in_with_fee
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

        let amount_out =
            self.get_amount_out(amount_in, reserve_in, reserve_out, pool.fee_rate_bps)?;

        if amount_out < min_amount_out {
            return Err(DefiError::Slippage {
                minimum_out: min_amount_out,
                actual_out: amount_out,
            });
        }

        // Update reserves.
        if is_a_to_b {
            pool.reserve_a = pool
                .reserve_a
                .checked_add(amount_in)
                .ok_or(DefiError::Overflow {
                    operation: "swap reserve_a increase".into(),
                })?;
            pool.reserve_b = pool
                .reserve_b
                .checked_sub(amount_out)
                .ok_or(DefiError::Overflow {
                    operation: "swap reserve_b decrease".into(),
                })?;
        } else {
            pool.reserve_b = pool
                .reserve_b
                .checked_add(amount_in)
                .ok_or(DefiError::Overflow {
                    operation: "swap reserve_b increase".into(),
                })?;
            pool.reserve_a = pool
                .reserve_a
                .checked_sub(amount_out)
                .ok_or(DefiError::Overflow {
                    operation: "swap reserve_a decrease".into(),
                })?;
        }

        Ok(amount_out)
    }

    /// Execute a swap and record a TWAP price observation after the reserves are updated.
    ///
    /// The post-swap spot price is computed from the new reserves as `reserve_b / reserve_a`
    /// (for an A→B swap) and pushed into the oracle accumulator identified by `pool_id`.
    ///
    /// The oracle must have been initialised for this pool/asset pair via
    /// [`TwapOracle::init_pool_asset`] before this method is called; if not, the swap
    /// still succeeds but no observation is recorded.
    #[allow(clippy::too_many_arguments)]
    pub fn execute_swap_with_twap(
        &self,
        pool: &mut LiquidityPool,
        amount_in: Amount,
        is_a_to_b: bool,
        min_amount_out: Amount,
        oracle: &mut TwapOracle,
        pool_id: &str,
        timestamp: Timestamp,
    ) -> DefiResult<Amount> {
        let amount_out = self.execute_swap(pool, amount_in, is_a_to_b, min_amount_out)?;

        // Derive post-swap spot price from updated reserves.
        // price = reserve_b / reserve_a (units of B per unit of A).
        if !pool.reserve_a.is_zero() && !pool.reserve_b.is_zero() {
            let spot = Price::new(pool.reserve_b.0, pool.reserve_a.0);
            // Record for asset_a (priced in asset_b).
            let asset_a: AssetId = pool.asset_a.clone();
            oracle.record_observation(pool_id, &asset_a, spot, timestamp);

            // Also record the inverse for asset_b (priced in asset_a).
            let inv_spot = Price::new(pool.reserve_a.0, pool.reserve_b.0);
            let asset_b: AssetId = pool.asset_b.clone();
            oracle.record_observation(pool_id, &asset_b, inv_spot, timestamp);
        }

        Ok(amount_out)
    }
}

impl Default for SwapEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wrfcoin_defi_common::types::{AssetId, PoolId};

    fn make_pool(reserve_a: u128, reserve_b: u128) -> LiquidityPool {
        let mut pool = LiquidityPool::new(PoolId::new(1), AssetId::new("A"), AssetId::new("B"), 30);
        pool.reserve_a = Amount::new(reserve_a);
        pool.reserve_b = Amount::new(reserve_b);
        pool.total_lp_supply = Amount::new(1000); // arbitrary
        pool
    }

    #[test]
    fn test_get_amount_out_basic() {
        let engine = SwapEngine::new();
        // Pool: 1000 A, 1000 B, 0.3% fee
        // Swap 100 A -> B
        // after_fee = 100 * 9970 = 997_000
        // numerator = 997_000 * 1000 = 997_000_000
        // denominator = 1000 * 10_000 + 997_000 = 10_997_000
        // out = 997_000_000 / 10_997_000 = 90
        let out = engine
            .get_amount_out(Amount::new(100), Amount::new(1000), Amount::new(1000), 30)
            .unwrap();
        assert_eq!(out, Amount::new(90));
    }

    #[test]
    fn test_get_amount_out_zero_input_fails() {
        let engine = SwapEngine::new();
        let result = engine.get_amount_out(Amount::ZERO, Amount::new(1000), Amount::new(1000), 30);
        assert!(result.is_err());
    }

    #[test]
    fn test_get_amount_out_empty_reserves_fails() {
        let engine = SwapEngine::new();
        let result = engine.get_amount_out(Amount::new(100), Amount::ZERO, Amount::new(1000), 30);
        assert!(result.is_err());
    }

    #[test]
    fn test_get_amount_in_basic() {
        let engine = SwapEngine::new();
        // Want 90 B from pool with 1000 A, 1000 B
        let amount_in = engine
            .get_amount_in(Amount::new(90), Amount::new(1000), Amount::new(1000), 30)
            .unwrap();
        // Should need ~100 A (plus rounding up)
        assert!(amount_in.0 >= 99 && amount_in.0 <= 101);
    }

    #[test]
    fn test_get_amount_in_exceeds_reserves_fails() {
        let engine = SwapEngine::new();
        let result =
            engine.get_amount_in(Amount::new(1000), Amount::new(100), Amount::new(500), 30);
        assert!(result.is_err());
    }

    #[test]
    fn test_execute_swap_a_to_b() {
        let engine = SwapEngine::new();
        let mut pool = make_pool(1000, 1000);
        let out = engine
            .execute_swap(&mut pool, Amount::new(100), true, Amount::ZERO)
            .unwrap();
        assert!(out.0 > 0);
        assert_eq!(pool.reserve_a, Amount::new(1100));
        assert_eq!(pool.reserve_b, Amount::new(1000 - out.0));
    }

    #[test]
    fn test_execute_swap_b_to_a() {
        let engine = SwapEngine::new();
        let mut pool = make_pool(1000, 1000);
        let out = engine
            .execute_swap(&mut pool, Amount::new(100), false, Amount::ZERO)
            .unwrap();
        assert!(out.0 > 0);
        assert_eq!(pool.reserve_b, Amount::new(1100));
        assert_eq!(pool.reserve_a, Amount::new(1000 - out.0));
    }

    #[test]
    fn test_execute_swap_slippage_protection() {
        let engine = SwapEngine::new();
        let mut pool = make_pool(1000, 1000);
        // Set min_amount_out higher than what we'd actually get
        let result = engine.execute_swap(&mut pool, Amount::new(100), true, Amount::new(999));
        assert!(result.is_err());
    }

    #[test]
    fn test_swap_preserves_invariant() {
        let engine = SwapEngine::new();
        let mut pool = make_pool(10_000, 10_000);
        let k_before = pool.invariant().unwrap();
        engine
            .execute_swap(&mut pool, Amount::new(500), true, Amount::ZERO)
            .unwrap();
        let k_after = pool.invariant().unwrap();
        // k should increase (fees are kept in the pool)
        assert!(k_after >= k_before);
    }
}
