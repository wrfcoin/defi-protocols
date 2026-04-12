use std::collections::BTreeMap;

/// A single tick in the concentrated liquidity pool.
///
/// Each initialized tick represents a price boundary where liquidity changes.
/// `liquidity_net` is positive when entering a range from below (crossing upward)
/// and negative when exiting (crossing downward).
#[derive(Debug, Clone, Default)]
pub struct Tick {
    /// Tick index (maps to a specific price via `tick_to_sqrt_price`).
    pub index: i32,
    /// Net liquidity change when this tick is crossed (positive = add, negative = remove).
    pub liquidity_net: i128,
    /// Fee growth of token0 *outside* this tick (accumulated globally before
    /// this tick was last crossed, or initialized).
    pub fee_growth_outside_0: u128,
    /// Fee growth of token1 *outside* this tick.
    pub fee_growth_outside_1: u128,
}

impl Tick {
    pub fn new(index: i32) -> Self {
        Self {
            index,
            liquidity_net: 0,
            fee_growth_outside_0: 0,
            fee_growth_outside_1: 0,
        }
    }

    /// Update fee_growth_outside values when this tick is crossed.
    /// Crossing flips the "outside" reference: outside becomes inside and vice versa.
    pub fn cross(&mut self, fee_growth_global_0: u128, fee_growth_global_1: u128) {
        self.fee_growth_outside_0 = fee_growth_global_0.wrapping_sub(self.fee_growth_outside_0);
        self.fee_growth_outside_1 = fee_growth_global_1.wrapping_sub(self.fee_growth_outside_1);
    }
}

/// Sparse bitmap tracking which ticks are initialized (have liquidity).
///
/// Uses a `BTreeMap<i32, bool>` for simplicity. In production this would use
/// a packed word bitmap for gas/compute efficiency.
#[derive(Debug, Clone, Default)]
pub struct TickBitmap {
    initialized: BTreeMap<i32, bool>,
}

impl TickBitmap {
    pub fn new() -> Self {
        Self::default()
    }

    /// Mark a tick as initialized.
    pub fn flip(&mut self, tick: i32) {
        let entry = self.initialized.entry(tick).or_insert(false);
        *entry = !*entry;
        if !*entry {
            self.initialized.remove(&tick);
        }
    }

    /// Returns true if the tick is initialized.
    pub fn is_initialized(&self, tick: i32) -> bool {
        self.initialized.get(&tick).copied().unwrap_or(false)
    }

    /// Find the next initialized tick at or after `from` (inclusive) when
    /// moving to higher ticks (`zero_for_one = false`), or the next initialized
    /// tick at or before `from` (inclusive) when moving to lower ticks
    /// (`zero_for_one = true`).
    ///
    /// Returns `None` if no initialized tick exists in the search direction.
    pub fn next_initialized_tick(&self, from: i32, zero_for_one: bool) -> Option<i32> {
        if zero_for_one {
            // Moving price down: look for highest tick < from
            self.initialized.range(..from).next_back().map(|(&t, _)| t)
        } else {
            // Moving price up: look for lowest tick > from
            self.initialized.range((from + 1)..).next().map(|(&t, _)| t)
        }
    }

    /// List all initialized ticks in ascending order.
    pub fn all_initialized(&self) -> Vec<i32> {
        self.initialized.keys().copied().collect()
    }
}

/// Compute the fee growth *inside* a tick range [lower, upper] from the
/// global fee growth and the per-tick fee growth outside values.
///
/// This follows the Uniswap V3 formula for fee accumulation inside a range.
pub fn fee_growth_inside(
    lower: &Tick,
    upper: &Tick,
    current_tick: i32,
    fee_growth_global_0: u128,
    fee_growth_global_1: u128,
) -> (u128, u128) {
    // fee_growth_below_lower: if current_tick >= lower.index, fee_growth_outside IS below;
    // otherwise it's above, so below = global - outside.
    let (fg_below_0, fg_below_1) = if current_tick >= lower.index {
        (lower.fee_growth_outside_0, lower.fee_growth_outside_1)
    } else {
        (
            fee_growth_global_0.wrapping_sub(lower.fee_growth_outside_0),
            fee_growth_global_1.wrapping_sub(lower.fee_growth_outside_1),
        )
    };

    // fee_growth_above_upper: if current_tick < upper.index, fee_growth_outside IS above;
    // otherwise above = global - outside.
    let (fg_above_0, fg_above_1) = if current_tick < upper.index {
        (upper.fee_growth_outside_0, upper.fee_growth_outside_1)
    } else {
        (
            fee_growth_global_0.wrapping_sub(upper.fee_growth_outside_0),
            fee_growth_global_1.wrapping_sub(upper.fee_growth_outside_1),
        )
    };

    // inside = global - below - above
    let inside_0 = fee_growth_global_0
        .wrapping_sub(fg_below_0)
        .wrapping_sub(fg_above_0);
    let inside_1 = fee_growth_global_1
        .wrapping_sub(fg_below_1)
        .wrapping_sub(fg_above_1);

    (inside_0, inside_1)
}

/// Compute the amount of token0 and token1 needed to provide `liquidity` units
/// into the range [sqrt_lower, sqrt_upper] given current sqrt price.
///
/// Based on Uniswap V3 liquidity math:
/// - If price is below range: only token0 required
///   `amount0 = liquidity * (sqrt_upper - sqrt_lower) / (sqrt_upper * sqrt_lower)`
/// - If price is above range: only token1 required
///   `amount1 = liquidity * (sqrt_upper - sqrt_lower)`
/// - If price is in range:
///   `amount0 = liquidity * (sqrt_upper - sqrt_current) / (sqrt_upper * sqrt_current)`
///   `amount1 = liquidity * (sqrt_current - sqrt_lower)`
///
/// All values are in Q64.64. Returns (amount0, amount1) as u128 base units.
pub fn liquidity_to_amounts(
    liquidity: u128,
    sqrt_price_current: u128,
    sqrt_price_lower: u128,
    sqrt_price_upper: u128,
) -> (u128, u128) {
    if sqrt_price_current <= sqrt_price_lower {
        // Price below range: only token0
        // amount0 = L * (sqrt_upper - sqrt_lower) / (sqrt_upper * sqrt_lower / Q64)
        // = L * (sqrt_upper - sqrt_lower) * Q64 / (sqrt_upper * sqrt_lower)
        let numerator = liquidity.saturating_mul(sqrt_price_upper.saturating_sub(sqrt_price_lower));
        // Divide by sqrt_lower first to reduce overflow, then by sqrt_upper
        let denom_lower = sqrt_price_lower.max(1);
        let denom_upper = sqrt_price_upper.max(1);
        let amount0 = (numerator / denom_lower)
            .checked_mul(1u128 << 32) // partial Q64 compensation
            .and_then(|v| v.checked_div(denom_upper >> 32))
            .unwrap_or_else(|| numerator / denom_lower / (denom_upper >> 32).max(1));
        (amount0, 0)
    } else if sqrt_price_current >= sqrt_price_upper {
        // Price above range: only token1
        // amount1 = L * (sqrt_upper - sqrt_lower) / Q64
        let amount1 = liquidity
            .saturating_mul(sqrt_price_upper.saturating_sub(sqrt_price_lower))
            / (1u128 << 32)  // scale down Q64 (use 2^32 steps to avoid overflow)
            / (1u128 << 32);
        (0, amount1)
    } else {
        // Price in range: both tokens
        // amount0 = L * (sqrt_upper - sqrt_current) / (sqrt_upper * sqrt_current / Q64)
        let num0 = liquidity.saturating_mul(sqrt_price_upper.saturating_sub(sqrt_price_current));
        let denom0_lower = sqrt_price_current.max(1);
        let denom0_upper = sqrt_price_upper.max(1);
        let amount0 = (num0 / denom0_lower)
            .checked_mul(1u128 << 32)
            .and_then(|v| v.checked_div(denom0_upper >> 32))
            .unwrap_or_else(|| num0 / denom0_lower / (denom0_upper >> 32).max(1));

        // amount1 = L * (sqrt_current - sqrt_lower) / Q64
        let amount1 = liquidity.saturating_mul(sqrt_price_current.saturating_sub(sqrt_price_lower))
            / (1u128 << 32)
            / (1u128 << 32);

        (amount0, amount1)
    }
}

/// Given a liquidity delta and tick range, compute the new sqrt price after
/// consuming `amount_in` of token0 (zero_for_one=true) or token1 (zero_for_one=false)
/// within the range [sqrt_lower, sqrt_upper].
///
/// Returns the new sqrt price after consuming up to `amount_in` within the range.
/// Also returns the actual amount consumed and amount output.
///
/// In-range constant product formula (within a tick range):
/// - For token0 in: `new_sqrt = (sqrt_current * sqrt_upper * L) / (L * sqrt_upper + amount_in * sqrt_current * sqrt_upper / Q64)`
/// - For token1 in: `new_sqrt = sqrt_current + amount_in * Q64 / L`
pub fn compute_swap_step(
    sqrt_price_current: u128,
    sqrt_price_target: u128,
    liquidity: u128,
    amount_remaining: u128,
    fee_bps: u64,
) -> (u128, u128, u128, u128) {
    // Returns (sqrt_price_next, amount_in, amount_out, fee_amount)
    if liquidity == 0 || amount_remaining == 0 {
        return (sqrt_price_current, 0, 0, 0);
    }

    let zero_for_one = sqrt_price_current >= sqrt_price_target;

    // Apply fee: amount_in_after_fee = amount_remaining * (10000 - fee_bps) / 10000
    let fee_complement = 10_000u128 - fee_bps as u128;
    let amount_after_fee = amount_remaining.saturating_mul(fee_complement) / 10_000;

    let (sqrt_price_next, amount_in, amount_out) = if zero_for_one {
        // Selling token0 → price decreases
        // new_sqrt = L * sqrt_current / (L + amount * sqrt_current / Q64)
        // We clamp to sqrt_price_target if we'd go past it.
        let amount_in_calc =
            amount_in_for_price_move_0(sqrt_price_current, sqrt_price_target, liquidity);
        if amount_after_fee >= amount_in_calc {
            // We'll reach the target price exactly
            let amount_out =
                amount_out_for_price_move_1(sqrt_price_current, sqrt_price_target, liquidity);
            (sqrt_price_target, amount_in_calc, amount_out)
        } else {
            // Partial fill: compute how far price moves with amount_after_fee of token0
            let next =
                new_sqrt_price_from_amount0(sqrt_price_current, liquidity, amount_after_fee, true);
            let amount_out = amount_out_for_price_move_1(sqrt_price_current, next, liquidity);
            (next, amount_after_fee, amount_out)
        }
    } else {
        // Selling token1 → price increases
        let amount_in_calc =
            amount_in_for_price_move_1(sqrt_price_current, sqrt_price_target, liquidity);
        if amount_after_fee >= amount_in_calc {
            let amount_out =
                amount_out_for_price_move_0(sqrt_price_current, sqrt_price_target, liquidity);
            (sqrt_price_target, amount_in_calc, amount_out)
        } else {
            let next =
                new_sqrt_price_from_amount1(sqrt_price_current, liquidity, amount_after_fee, false);
            let amount_out = amount_out_for_price_move_0(sqrt_price_current, next, liquidity);
            (next, amount_after_fee, amount_out)
        }
    };

    // fee_amount = amount_in * fee_bps / (10000 - fee_bps)  (on top of the amount)
    let fee_amount = amount_in.saturating_mul(fee_bps as u128) / fee_complement.max(1);

    (sqrt_price_next, amount_in, amount_out, fee_amount)
}

/// Amount of token0 required to move sqrt_price from `sqrt_a` down to `sqrt_b` (a > b).
/// Formula: L * (sqrt_a - sqrt_b) / (sqrt_a * sqrt_b / Q64)
///        = L * (sqrt_a - sqrt_b) * Q64 / (sqrt_a * sqrt_b)
fn amount_in_for_price_move_0(sqrt_a: u128, sqrt_b: u128, liquidity: u128) -> u128 {
    if sqrt_a <= sqrt_b || sqrt_b == 0 {
        return 0;
    }
    let diff = sqrt_a - sqrt_b;
    // L * diff * Q64 / (sqrt_a * sqrt_b) — avoid overflow by dividing step-by-step
    // = L * diff / sqrt_b * Q64 / sqrt_a  (order chosen to minimize overflow)
    let step1 = liquidity.saturating_mul(diff);
    // divide by sqrt_b (Q64 units) to get units, then multiply back partial Q64
    let step2 = (step1 / (sqrt_b >> 32).max(1)) << 32;
    step2 / (sqrt_a >> 32).max(1)
}

/// Amount of token1 required to move sqrt_price from `sqrt_a` up to `sqrt_b` (b > a).
/// Formula: L * (sqrt_b - sqrt_a) / Q64
fn amount_in_for_price_move_1(sqrt_a: u128, sqrt_b: u128, liquidity: u128) -> u128 {
    if sqrt_b <= sqrt_a {
        return 0;
    }
    let diff = sqrt_b - sqrt_a;
    // L * diff / Q64
    liquidity.saturating_mul(diff) >> 64
}

/// Amount of token1 output when price moves from `sqrt_a` down to `sqrt_b`.
fn amount_out_for_price_move_1(sqrt_a: u128, sqrt_b: u128, liquidity: u128) -> u128 {
    amount_in_for_price_move_1(sqrt_b, sqrt_a, liquidity)
}

/// Amount of token0 output when price moves from `sqrt_a` up to `sqrt_b`.
fn amount_out_for_price_move_0(sqrt_a: u128, sqrt_b: u128, liquidity: u128) -> u128 {
    amount_in_for_price_move_0(sqrt_b, sqrt_a, liquidity)
}

/// Compute new sqrt_price after adding `amount` of token0 to a range.
/// If `add` is true (selling token0), price decreases.
fn new_sqrt_price_from_amount0(
    sqrt_price: u128,
    liquidity: u128,
    amount: u128,
    _add: bool,
) -> u128 {
    // new_sqrt = L * sqrt_price / (L + amount * sqrt_price / Q64)
    // = L * Q64 / (L * Q64 / sqrt_price + amount)
    if liquidity == 0 {
        return sqrt_price;
    }
    let numerator = (liquidity << 32).saturating_mul(1u128 << 32); // L * Q64
    let denom_part = liquidity.saturating_mul(1u128 << 64) / sqrt_price.max(1);
    let denom = denom_part.saturating_add(amount);
    if denom == 0 {
        return sqrt_price;
    }
    numerator / denom
}

/// Compute new sqrt_price after adding `amount` of token1 to a range.
/// Price increases.
fn new_sqrt_price_from_amount1(
    sqrt_price: u128,
    liquidity: u128,
    amount: u128,
    _add: bool,
) -> u128 {
    // new_sqrt = sqrt_price + amount * Q64 / L
    if liquidity == 0 {
        return sqrt_price;
    }
    let delta = amount.checked_shl(64).unwrap_or(u128::MAX) / liquidity;
    sqrt_price.saturating_add(delta)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tick_cross_flips_outside() {
        let mut tick = Tick::new(10);
        tick.fee_growth_outside_0 = 100;
        tick.fee_growth_outside_1 = 200;
        tick.cross(500, 1000);
        assert_eq!(tick.fee_growth_outside_0, 400); // 500 - 100
        assert_eq!(tick.fee_growth_outside_1, 800); // 1000 - 200
    }

    #[test]
    fn test_tick_bitmap_flip() {
        let mut bm = TickBitmap::new();
        assert!(!bm.is_initialized(5));
        bm.flip(5);
        assert!(bm.is_initialized(5));
        bm.flip(5); // flip back
        assert!(!bm.is_initialized(5));
    }

    #[test]
    fn test_tick_bitmap_next_initialized() {
        let mut bm = TickBitmap::new();
        bm.flip(10);
        bm.flip(20);
        bm.flip(-5);

        // Moving up from 5: next initialized above 5 is 10
        assert_eq!(bm.next_initialized_tick(5, false), Some(10));
        // Moving up from 10: next is 20
        assert_eq!(bm.next_initialized_tick(10, false), Some(20));
        // Moving down from 15: next below 15 is 10
        assert_eq!(bm.next_initialized_tick(15, true), Some(10));
        // Moving down from 5: next below 5 is -5
        assert_eq!(bm.next_initialized_tick(5, true), Some(-5));
        // No tick above 30
        assert_eq!(bm.next_initialized_tick(30, false), None);
    }

    #[test]
    fn test_fee_growth_inside_current_in_range() {
        let lower = Tick {
            index: 0,
            liquidity_net: 0,
            fee_growth_outside_0: 10,
            fee_growth_outside_1: 20,
        };
        let upper = Tick {
            index: 100,
            liquidity_net: 0,
            fee_growth_outside_0: 5,
            fee_growth_outside_1: 10,
        };
        // current_tick = 50 (in range): fee_growth_global = 100, 200
        // below = lower.outside (since current >= lower.index) = (10, 20)
        // above = upper.outside (since current < upper.index) = (5, 10)
        // inside = global - below - above = (85, 170)
        let (fg0, fg1) = fee_growth_inside(&lower, &upper, 50, 100, 200);
        assert_eq!(fg0, 85);
        assert_eq!(fg1, 170);
    }

    #[test]
    fn test_liquidity_to_amounts_below_range() {
        use crate::concentrated::sqrt_price::tick_to_sqrt_price;
        let lower = tick_to_sqrt_price(10).0;
        let upper = tick_to_sqrt_price(20).0;
        let current = tick_to_sqrt_price(5).0; // below range
        let (amount0, amount1) = liquidity_to_amounts(1_000_000, current, lower, upper);
        assert!(amount0 > 0, "should need token0 when price is below range");
        assert_eq!(
            amount1, 0,
            "should need zero token1 when price is below range"
        );
    }

    #[test]
    fn test_liquidity_to_amounts_above_range() {
        use crate::concentrated::sqrt_price::tick_to_sqrt_price;
        let lower = tick_to_sqrt_price(10).0;
        let upper = tick_to_sqrt_price(20).0;
        let current = tick_to_sqrt_price(25).0; // above range
        let (amount0, amount1) = liquidity_to_amounts(1_000_000, current, lower, upper);
        assert_eq!(
            amount0, 0,
            "should need zero token0 when price is above range"
        );
        assert!(amount1 > 0, "should need token1 when price is above range");
    }

    #[test]
    fn test_compute_swap_step_zero_liquidity() {
        let (next, in_, out, fee) = compute_swap_step(100, 90, 0, 1000, 30);
        assert_eq!(next, 100);
        assert_eq!(in_, 0);
        assert_eq!(out, 0);
        assert_eq!(fee, 0);
    }
}
