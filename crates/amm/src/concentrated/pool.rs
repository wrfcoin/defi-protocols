use std::collections::HashMap;

use wrfcoin_defi_common::{
    error::{DefiError, DefiResult},
    types::Address,
};

use crate::fees::FeeTier;

use super::{
    position::{ConcentratedPosition, PositionId},
    sqrt_price::{tick_to_sqrt_price, SqrtPrice},
    tick::{compute_swap_step, fee_growth_inside, liquidity_to_amounts, Tick, TickBitmap},
};

/// A concentrated liquidity pool in the style of Uniswap V3.
///
/// Unlike the uniform `LiquidityPool`, this pool tracks liquidity per tick
/// range. Only LPs whose range brackets the current price earn fees.
///
/// This is a standalone pool type; it does NOT replace the existing
/// `LiquidityPool` (constant-product x*y=k). It is a future extension of
/// the AMM system and may eventually implement the `IWrfAMM` Solidity
/// interface described in `contracts/interfaces/IWrfAMM.sol`.
///
/// # Architecture note
/// The Uniswap V3 math uses Q64.96 (requires u256). This implementation
/// uses Q64.64 (fits in u128) — see `sqrt_price.rs` for rationale.
#[derive(Debug)]
pub struct ConcentratedLiquidityPool {
    /// Current sqrt price in Q64.64 fixed-point.
    pub sqrt_price_current: SqrtPrice,
    /// Current active tick (the highest initialized tick below the current price).
    pub current_tick: i32,
    /// Total virtual liquidity active at the current price.
    pub liquidity: u128,
    /// Sparse map of initialized ticks.
    pub tick_map: HashMap<i32, Tick>,
    /// Bitmap tracking which ticks are initialized.
    pub tick_bitmap: TickBitmap,
    /// All LP positions indexed by position ID.
    pub positions: HashMap<PositionId, ConcentratedPosition>,
    /// Fee tier for this pool.
    pub fee_tier: FeeTier,
    /// Global fee growth per unit of liquidity for token0 (Q64.64 per-unit).
    pub fee_growth_global_0: u128,
    /// Global fee growth per unit of liquidity for token1 (Q64.64 per-unit).
    pub fee_growth_global_1: u128,
    /// Next position ID to assign.
    next_position_id: u64,
    /// Protocol fee accumulation for token0.
    pub protocol_fees_0: u128,
    /// Protocol fee accumulation for token1.
    pub protocol_fees_1: u128,
}

impl ConcentratedLiquidityPool {
    /// Create a new pool starting at `initial_tick`.
    pub fn new(initial_tick: i32, fee_tier: FeeTier) -> Self {
        let sqrt_price = tick_to_sqrt_price(initial_tick);
        Self {
            sqrt_price_current: sqrt_price,
            current_tick: initial_tick,
            liquidity: 0,
            tick_map: HashMap::new(),
            tick_bitmap: TickBitmap::new(),
            positions: HashMap::new(),
            fee_tier,
            fee_growth_global_0: 0,
            fee_growth_global_1: 0,
            next_position_id: 0,
            protocol_fees_0: 0,
            protocol_fees_1: 0,
        }
    }

    /// Get or create a mutable reference to a tick.
    fn get_or_create_tick(&mut self, index: i32) -> &mut Tick {
        self.tick_map
            .entry(index)
            .or_insert_with(|| Tick::new(index))
    }

    /// Get the current fee growth inside a tick range.
    fn current_fee_growth_inside(&self, lower_tick: i32, upper_tick: i32) -> (u128, u128) {
        let lower = self
            .tick_map
            .get(&lower_tick)
            .cloned()
            .unwrap_or_else(|| Tick::new(lower_tick));
        let upper = self
            .tick_map
            .get(&upper_tick)
            .cloned()
            .unwrap_or_else(|| Tick::new(upper_tick));
        fee_growth_inside(
            &lower,
            &upper,
            self.current_tick,
            self.fee_growth_global_0,
            self.fee_growth_global_1,
        )
    }

    /// Add liquidity to a tick range for an owner.
    ///
    /// Returns `(amount0, amount1)` — the token amounts the caller must deposit.
    ///
    /// If the current price is:
    /// - Below the range: only token0 is required (range order / single-sided).
    /// - Above the range: only token1 is required (range order / single-sided).
    /// - Inside the range: both tokens are required.
    pub fn mint(
        &mut self,
        owner: Address,
        lower_tick: i32,
        upper_tick: i32,
        liquidity_delta: u128,
    ) -> DefiResult<(u128, u128, PositionId)> {
        if lower_tick >= upper_tick {
            return Err(DefiError::InvalidAmount {
                reason: "lower_tick must be strictly less than upper_tick".into(),
            });
        }
        if liquidity_delta == 0 {
            return Err(DefiError::InvalidAmount {
                reason: "liquidity_delta must be non-zero".into(),
            });
        }

        // Compute token amounts required.
        let (amount0, amount1) = liquidity_to_amounts(
            liquidity_delta,
            self.sqrt_price_current.0,
            tick_to_sqrt_price(lower_tick).0,
            tick_to_sqrt_price(upper_tick).0,
        );

        // Get current fee growth inside the range.
        let (fg_inside_0, fg_inside_1) = self.current_fee_growth_inside(lower_tick, upper_tick);

        // Update lower tick: liquidity_net increases (we add liquidity above this tick).
        {
            let lower = self.get_or_create_tick(lower_tick);
            lower.liquidity_net = lower
                .liquidity_net
                .checked_add(liquidity_delta as i128)
                .ok_or_else(|| DefiError::Overflow {
                    operation: "tick liquidity_net lower".into(),
                })?;
        }
        self.tick_bitmap.flip(lower_tick);
        // Flip back if it was already initialized — we only add; track net parity.
        // Simpler: always ensure the tick is in the bitmap once liquidity is non-zero.
        // We'll use the tick_map to determine initialization.
        // Re-initialize the bitmap entry correctly.
        // (tick_bitmap.flip toggles; calling flip again would remove it — use set directly)
        // Actually we already flipped above. If it was uninitialized (default), one flip = initialized.
        // If it was already initialized (from a prior mint), we flipped it off — flip again to restore.
        if !self.tick_bitmap.is_initialized(lower_tick) {
            self.tick_bitmap.flip(lower_tick);
        }

        // Update upper tick: liquidity_net decreases (we remove liquidity above this tick).
        {
            let upper = self.get_or_create_tick(upper_tick);
            upper.liquidity_net = upper
                .liquidity_net
                .checked_sub(liquidity_delta as i128)
                .ok_or_else(|| DefiError::Overflow {
                    operation: "tick liquidity_net upper".into(),
                })?;
        }
        if !self.tick_bitmap.is_initialized(upper_tick) {
            self.tick_bitmap.flip(upper_tick);
        }

        // If the current price is within this range, add to active liquidity.
        if self.current_tick >= lower_tick && self.current_tick < upper_tick {
            self.liquidity =
                self.liquidity
                    .checked_add(liquidity_delta)
                    .ok_or_else(|| DefiError::Overflow {
                        operation: "pool liquidity".into(),
                    })?;
        }

        // Create or update the position.
        let pos_id = PositionId::new(self.next_position_id);
        self.next_position_id += 1;

        let mut position = ConcentratedPosition::new(owner, lower_tick, upper_tick);
        position.fee_growth_inside_last_0 = fg_inside_0;
        position.fee_growth_inside_last_1 = fg_inside_1;
        position
            .update(liquidity_delta as i128, fg_inside_0, fg_inside_1)
            .map_err(|e| DefiError::InvalidAmount { reason: e.into() })?;

        self.positions.insert(pos_id, position);

        Ok((amount0, amount1, pos_id))
    }

    /// Remove liquidity from a position.
    ///
    /// Returns `(amount0, amount1, fees_earned_0, fees_earned_1)`.
    pub fn burn(
        &mut self,
        position_id: PositionId,
        liquidity_delta: u128,
    ) -> DefiResult<(u128, u128, u128, u128)> {
        let pos = self
            .positions
            .get(&position_id)
            .ok_or_else(|| DefiError::InvalidAmount {
                reason: "position not found".into(),
            })?;

        if liquidity_delta > pos.liquidity {
            return Err(DefiError::InsufficientBalance {
                required: wrfcoin_defi_common::types::Amount::new(liquidity_delta),
                available: wrfcoin_defi_common::types::Amount::new(pos.liquidity),
            });
        }

        let lower_tick = pos.lower_tick;
        let upper_tick = pos.upper_tick;

        // Compute token amounts returned.
        let (amount0, amount1) = liquidity_to_amounts(
            liquidity_delta,
            self.sqrt_price_current.0,
            tick_to_sqrt_price(lower_tick).0,
            tick_to_sqrt_price(upper_tick).0,
        );

        // Get current fee growth inside.
        let (fg_inside_0, fg_inside_1) = self.current_fee_growth_inside(lower_tick, upper_tick);

        // Update position (accrues fees, reduces liquidity).
        let pos = self.positions.get_mut(&position_id).unwrap();
        pos.update(-(liquidity_delta as i128), fg_inside_0, fg_inside_1)
            .map_err(|e| DefiError::InvalidAmount { reason: e.into() })?;
        let (fees_0, fees_1) = pos.collect_fees();

        // Update tick liquidity_net.
        {
            let lower = self.get_or_create_tick(lower_tick);
            lower.liquidity_net -= liquidity_delta as i128;
        }
        {
            let upper = self.get_or_create_tick(upper_tick);
            upper.liquidity_net += liquidity_delta as i128;
        }

        // If the current price was in this range, reduce active liquidity.
        if self.current_tick >= lower_tick && self.current_tick < upper_tick {
            self.liquidity = self.liquidity.saturating_sub(liquidity_delta);
        }

        Ok((amount0, amount1, fees_0, fees_1))
    }

    /// Execute a swap through the pool, stepping through tick boundaries.
    ///
    /// `zero_for_one`: if true, swap token0 for token1 (price decreases).
    ///                 if false, swap token1 for token0 (price increases).
    /// `amount_in`: exact input amount.
    ///
    /// Returns `amount_out` (the tokens received by the caller).
    pub fn swap(&mut self, amount_in: u128, zero_for_one: bool) -> DefiResult<u128> {
        if amount_in == 0 {
            return Err(DefiError::InvalidAmount {
                reason: "swap amount must be non-zero".into(),
            });
        }
        if self.liquidity == 0 {
            return Err(DefiError::InsufficientLiquidity {
                requested: wrfcoin_defi_common::types::Amount::new(amount_in),
                available: wrfcoin_defi_common::types::Amount::ZERO,
            });
        }

        let fee_bps = self.fee_tier.as_bps();
        let mut amount_remaining = amount_in;
        let mut total_out: u128 = 0;

        // Iterate through ticks until the input is fully consumed.
        for _step in 0..200 {
            if amount_remaining == 0 {
                break;
            }

            // Find the next tick boundary in the direction of the swap.
            let next_tick = if zero_for_one {
                // Price going down: look for the next initialized tick below current.
                self.tick_bitmap
                    .next_initialized_tick(self.current_tick, true)
                    .unwrap_or(-500) // fallback to min tick
            } else {
                // Price going up: look for the next initialized tick above current.
                self.tick_bitmap
                    .next_initialized_tick(self.current_tick, false)
                    .unwrap_or(500) // fallback to max tick
            };

            let sqrt_price_target = tick_to_sqrt_price(next_tick).0;

            // Execute one swap step within the current tick range.
            let (sqrt_price_next, amount_in_used, amount_out, fee_amount) = compute_swap_step(
                self.sqrt_price_current.0,
                sqrt_price_target,
                self.liquidity,
                amount_remaining,
                fee_bps,
            );

            // Update pool state.
            self.sqrt_price_current = SqrtPrice(sqrt_price_next);

            // Accumulate fee growth per unit of liquidity (Q64.64 per-liquidity).
            if self.liquidity > 0 {
                let fee_per_unit = (fee_amount << 64) / self.liquidity;
                if zero_for_one {
                    self.fee_growth_global_0 = self.fee_growth_global_0.wrapping_add(fee_per_unit);
                } else {
                    self.fee_growth_global_1 = self.fee_growth_global_1.wrapping_add(fee_per_unit);
                }
            }

            amount_remaining = amount_remaining.saturating_sub(amount_in_used + fee_amount);
            total_out = total_out.saturating_add(amount_out);

            // If we reached the target tick, cross it and update liquidity.
            if sqrt_price_next == sqrt_price_target {
                let tick_to_cross = next_tick;
                if let Some(tick) = self.tick_map.get_mut(&tick_to_cross) {
                    let lnet = tick.liquidity_net;
                    tick.cross(self.fee_growth_global_0, self.fee_growth_global_1);
                    // When crossing a tick going down, we subtract liquidity_net.
                    // When crossing going up, we add liquidity_net.
                    if zero_for_one {
                        // Moving below tick: subtract liquidity_net
                        if lnet >= 0 {
                            self.liquidity = self.liquidity.saturating_sub(lnet as u128);
                        } else {
                            self.liquidity = self.liquidity.saturating_add((-lnet) as u128);
                        }
                        self.current_tick = next_tick - 1;
                    } else {
                        // Moving above tick: add liquidity_net
                        if lnet >= 0 {
                            self.liquidity = self.liquidity.saturating_add(lnet as u128);
                        } else {
                            self.liquidity = self.liquidity.saturating_sub((-lnet) as u128);
                        }
                        self.current_tick = next_tick;
                    }
                } else {
                    // No tick data at this boundary; just update current_tick.
                    if zero_for_one {
                        self.current_tick = next_tick - 1;
                    } else {
                        self.current_tick = next_tick;
                    }
                }

                // If no more active liquidity, stop.
                if self.liquidity == 0 {
                    break;
                }
            } else {
                // Didn't reach the target tick; price is within the range. Done.
                // Update current_tick to match the new price.
                self.current_tick = super::sqrt_price::sqrt_price_to_tick(self.sqrt_price_current);
                break;
            }
        }

        Ok(total_out)
    }

    /// Get the current price as a SqrtPrice.
    pub fn current_sqrt_price(&self) -> SqrtPrice {
        self.sqrt_price_current
    }

    /// Get the active liquidity at the current price.
    pub fn active_liquidity(&self) -> u128 {
        self.liquidity
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owner(byte: u8) -> Address {
        Address::from_bytes([byte; 32])
    }

    /// Helper to create a pool at tick 0 with 30bps fee.
    fn pool() -> ConcentratedLiquidityPool {
        ConcentratedLiquidityPool::new(0, FeeTier::ThirtyBasicPoints)
    }

    #[test]
    fn test_pool_initial_state() {
        let p = pool();
        assert_eq!(p.current_tick, 0);
        assert_eq!(p.liquidity, 0);
        assert_eq!(p.sqrt_price_current, SqrtPrice::ONE);
    }

    #[test]
    fn test_mint_stores_position() {
        let mut p = pool();
        let (a0, a1, pos_id) = p.mint(owner(1), -10, 10, 1_000_000).unwrap();
        // Current tick = 0 is inside [-10, 10], so both tokens needed
        // (amounts may be small due to Q64.64 arithmetic, but both should be >= 0)
        let _ = (a0, a1); // amounts are checked in integration tests below
        assert!(p.positions.contains_key(&pos_id));
        assert_eq!(p.positions[&pos_id].liquidity, 1_000_000);
    }

    #[test]
    fn test_mint_active_liquidity_updated() {
        let mut p = pool();
        p.mint(owner(1), -10, 10, 1_000_000).unwrap();
        // current_tick = 0, which is in range [-10, 10)
        assert_eq!(p.liquidity, 1_000_000);
    }

    #[test]
    fn test_mint_below_range_no_token1() {
        // Pool at tick 0; LP provides range [10, 20] — price is below range
        let mut p = pool();
        let (a0, a1, _) = p.mint(owner(1), 10, 20, 1_000_000).unwrap();
        assert!(a0 > 0, "token0 required when price below range");
        assert_eq!(a1, 0, "no token1 when price below range (range order)");
        // Liquidity should NOT be added to active pool (current_tick=0 < lower=10)
        assert_eq!(p.liquidity, 0);
    }

    #[test]
    fn test_mint_above_range_no_token0() {
        // Pool at tick 20; LP provides range [0, 10] — price is above range
        let mut p = ConcentratedLiquidityPool::new(20, FeeTier::ThirtyBasicPoints);
        let (a0, a1, _) = p.mint(owner(1), 0, 10, 1_000_000).unwrap();
        assert_eq!(a0, 0, "no token0 when price above range (range order)");
        assert!(a1 > 0, "token1 required when price above range");
    }

    #[test]
    fn test_mint_invalid_tick_order_fails() {
        let mut p = pool();
        let result = p.mint(owner(1), 10, 5, 1_000_000); // lower >= upper
        assert!(result.is_err());
    }

    #[test]
    fn test_mint_zero_liquidity_fails() {
        let mut p = pool();
        let result = p.mint(owner(1), -10, 10, 0);
        assert!(result.is_err());
    }

    #[test]
    fn test_swap_no_liquidity_fails() {
        let mut p = pool();
        let result = p.swap(1000, true);
        assert!(result.is_err());
    }

    #[test]
    fn test_swap_in_range() {
        let mut p = pool();
        // Add wide range liquidity
        p.mint(owner(1), -100, 100, 1_000_000_000).unwrap();

        let price_before = p.sqrt_price_current;
        let out = p.swap(10_000, true).unwrap(); // swap token0 for token1
        let price_after = p.sqrt_price_current;

        assert!(out > 0, "swap should produce token1 output");
        assert!(
            price_after < price_before,
            "price should decrease when selling token0 (zero_for_one=true)"
        );
    }

    #[test]
    fn test_swap_price_moves_opposite_directions() {
        let mut p = pool();
        p.mint(owner(1), -100, 100, 1_000_000_000).unwrap();

        let price_mid = p.sqrt_price_current;

        // Swap token0 → token1: price goes down
        p.swap(10_000, true).unwrap();
        let price_down = p.sqrt_price_current;
        assert!(price_down < price_mid);

        // Restore and swap token1 → token0: price goes up
        let mut p2 = pool();
        p2.mint(owner(1), -100, 100, 1_000_000_000).unwrap();
        p2.swap(10_000, false).unwrap();
        let price_up = p2.sqrt_price_current;
        assert!(price_up > price_mid);
    }

    #[test]
    fn test_burn_returns_liquidity() {
        let mut p = pool();
        let (_, _, pos_id) = p.mint(owner(1), -10, 10, 1_000_000).unwrap();

        let (a0, a1, fees0, fees1) = p.burn(pos_id, 500_000).unwrap();
        // Half the liquidity removed; some tokens should come back
        let _ = (a0, a1, fees0, fees1); // detailed amounts tested in integration test
        assert_eq!(p.positions[&pos_id].liquidity, 500_000);
        assert_eq!(p.liquidity, 500_000);
    }

    #[test]
    fn test_burn_too_much_fails() {
        let mut p = pool();
        let (_, _, pos_id) = p.mint(owner(1), -10, 10, 1_000_000).unwrap();
        let result = p.burn(pos_id, 2_000_000);
        assert!(result.is_err());
    }

    #[test]
    fn test_burn_nonexistent_position_fails() {
        let mut p = pool();
        let result = p.burn(PositionId::new(999), 1_000);
        assert!(result.is_err());
    }

    #[test]
    fn test_fee_growth_on_swap() {
        let mut p = pool();
        p.mint(owner(1), -100, 100, 1_000_000_000).unwrap();

        let fg0_before = p.fee_growth_global_0;
        p.swap(100_000, true).unwrap(); // token0 in → fees in token0
        let fg0_after = p.fee_growth_global_0;

        assert!(
            fg0_after > fg0_before,
            "fee_growth_global_0 should increase after swap"
        );
    }

    #[test]
    fn test_fee_accumulated_for_lp_in_range() {
        let mut p = pool();
        let (_, _, pos_id) = p.mint(owner(1), -100, 100, 1_000_000_000).unwrap();

        // Execute a swap to generate fees
        p.swap(1_000_000, true).unwrap();

        // Burn 0 liquidity to trigger fee accrual
        let (_, _, fees_0, fees_1) = p.burn(pos_id, 0).unwrap();
        // LP was in the active range, so fees_0 should be > 0
        // (fees_1 = 0 since we swapped token0, generating token0 fees only)
        assert!(
            fees_0 > 0,
            "LP in active range should earn token0 fees after token0→token1 swap"
        );
        let _ = fees_1;
    }

    #[test]
    fn test_fee_not_accumulated_for_lp_out_of_range() {
        let mut p = pool();
        // LP provides liquidity in range [50, 100] — price is at tick 0 (below range)
        let (_, _, pos_id) = p.mint(owner(1), 50, 100, 1_000_000_000).unwrap();

        // Add liquidity in the active range so swaps can happen
        p.mint(owner(2), -100, 49, 1_000_000_000).unwrap();

        // Swap token0 → token1 within the active range [-100, 49]
        p.swap(100_000, true).unwrap();

        // LP in [50, 100] should NOT have earned fees (price never entered their range)
        let (_, _, fees_0, fees_1) = p.burn(pos_id, 0).unwrap();
        // Fees should be 0 (or negligibly small due to rounding)
        assert_eq!(fees_0, 0, "LP out of range should not earn fees");
        assert_eq!(fees_1, 0, "LP out of range should not earn fees");
    }
}
