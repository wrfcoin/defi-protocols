/// Concentrated liquidity swap utilities.
///
/// This module re-exports the low-level swap math from `tick.rs` and provides
/// higher-level helpers for testing and integration.
///
/// The core swap logic (`compute_swap_step`) lives in `tick.rs` alongside the
/// tick math that drives it. This module is a thin wrapper that documents the
/// swap execution model and provides any additional helpers needed by callers.

pub use crate::concentrated::tick::compute_swap_step;

/// Describes the result of a single swap step across one tick range.
#[derive(Debug, Clone, Copy)]
pub struct SwapStep {
    /// The sqrt price after this step.
    pub sqrt_price_next: u128,
    /// Token input consumed in this step (excluding fee).
    pub amount_in: u128,
    /// Token output produced in this step.
    pub amount_out: u128,
    /// Fee charged on the input in this step.
    pub fee_amount: u128,
}

impl SwapStep {
    /// Execute one swap step and return the structured result.
    pub fn execute(
        sqrt_price_current: u128,
        sqrt_price_target: u128,
        liquidity: u128,
        amount_remaining: u128,
        fee_bps: u64,
    ) -> Self {
        let (sqrt_price_next, amount_in, amount_out, fee_amount) = compute_swap_step(
            sqrt_price_current,
            sqrt_price_target,
            liquidity,
            amount_remaining,
            fee_bps,
        );
        Self {
            sqrt_price_next,
            amount_in,
            amount_out,
            fee_amount,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::concentrated::sqrt_price::{tick_to_sqrt_price, Q64};

    #[test]
    fn test_swap_step_in_range_produces_output() {
        // Pool at price 1.0 (tick 0), target price at tick -10 (lower)
        let current = tick_to_sqrt_price(0).0;
        let target = tick_to_sqrt_price(-10).0;
        let liquidity = 1_000_000_000u128;
        let amount_in = 100_000u128;

        let step = SwapStep::execute(current, target, liquidity, amount_in, 30);
        // Should produce some output and price should move toward target
        assert!(step.amount_out > 0, "swap step should produce output");
        assert!(
            step.sqrt_price_next <= current,
            "price should decrease (selling token0)"
        );
        assert!(
            step.sqrt_price_next >= target,
            "price should not go past the tick boundary"
        );
        assert!(step.fee_amount > 0, "fee should be charged");
    }

    #[test]
    fn test_swap_step_reaches_target() {
        // With a huge amount_in, the step should hit the target exactly.
        let current = tick_to_sqrt_price(0).0;
        let target = tick_to_sqrt_price(-1).0;
        let liquidity = 1_000_000u128;
        let amount_in = u128::MAX / 2; // More than enough

        let step = SwapStep::execute(current, target, liquidity, amount_in, 30);
        assert_eq!(
            step.sqrt_price_next, target,
            "with excess input, should hit target exactly"
        );
    }

    #[test]
    fn test_swap_step_token1_in() {
        // Selling token1 → price increases
        let current = tick_to_sqrt_price(0).0;
        let target = tick_to_sqrt_price(10).0; // higher price
        let liquidity = 1_000_000_000u128;
        let amount_in = 100_000u128;

        let step = SwapStep::execute(current, target, liquidity, amount_in, 30);
        assert!(step.amount_out > 0);
        assert!(
            step.sqrt_price_next >= current,
            "price should increase when selling token1"
        );
        assert!(
            step.sqrt_price_next <= target,
            "price should not exceed tick boundary"
        );
    }

    #[test]
    fn test_swap_step_zero_liquidity() {
        let step = SwapStep::execute(Q64, Q64 - 1000, 0, 1000, 30);
        assert_eq!(step.amount_in, 0);
        assert_eq!(step.amount_out, 0);
        assert_eq!(step.fee_amount, 0);
    }

    #[test]
    fn test_fee_is_proportional_to_input() {
        let current = tick_to_sqrt_price(0).0;
        let target = tick_to_sqrt_price(-5).0;
        let liquidity = 1_000_000_000u128;

        let step1 = SwapStep::execute(current, target, liquidity, 10_000, 30);
        let step2 = SwapStep::execute(current, target, liquidity, 20_000, 30);

        // Double the input → roughly double the fee (within rounding)
        if step1.fee_amount > 0 {
            let ratio = step2.fee_amount / step1.fee_amount;
            assert!(ratio >= 1 && ratio <= 3, "fee should scale with input");
        }
    }
}
