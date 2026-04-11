/// LMSR (Logarithmic Market Scoring Rule) cost function implementation.
///
/// # Why f64?
/// The LMSR cost function C(q) = b·ln(Σexp(qᵢ/b)) requires transcendental
/// operations (ln, exp) that have no exact integer or fixed-point representation.
/// We use f64 throughout the LMSR module for pricing calculations.  On-chain
/// settlement amounts are converted back to u128 base units via `lmsr_cost_to_amount`
/// before touching the `Amount` type, keeping the financial accounting layer in
/// exact integer arithmetic.  The maximum rounding error on a single trade is
/// one WRF base unit (< 10⁻⁸ WRF), which is acceptable for a prediction market.
use crate::error::{PredictionError, PredictionResult};

/// Compute the LMSR cost function value.
///
/// C(q) = b · ln(Σᵢ exp(qᵢ / b))
///
/// Uses the log-sum-exp trick for numerical stability:
///   max_val = max(qᵢ / b)
///   C(q)    = b · (max_val + ln(Σᵢ exp(qᵢ/b − max_val)))
pub fn cost(q: &[f64], b: f64) -> PredictionResult<f64> {
    if !b.is_finite() {
        return Err(PredictionError::NonFinite);
    }
    for &qi in q {
        if !qi.is_finite() {
            return Err(PredictionError::NonFinite);
        }
    }
    if b <= 0.0 {
        return Err(PredictionError::LmsrError {
            reason: "liquidity parameter b must be positive".into(),
        });
    }
    if q.is_empty() {
        return Err(PredictionError::LmsrError {
            reason: "outcome vector q must not be empty".into(),
        });
    }

    // Log-sum-exp trick: shift by max to prevent exp overflow.
    let max_val = q.iter().cloned().fold(f64::NEG_INFINITY, f64::max) / b;
    let sum_exp: f64 = q.iter().map(|&qi| (qi / b - max_val).exp()).sum();
    Ok(b * (max_val + sum_exp.ln()))
}

/// Compute the marginal cost of buying `delta` shares of outcome `i`.
///
/// Returns the net WRF cost (in f64 base units) the trader must pay:
///   marginal_cost = C(q') − C(q)
/// where q'[i] = q[i] + delta and q'[j] = q[j] for j ≠ i.
pub fn buy_cost(q: &[f64], b: f64, outcome_index: usize, delta: f64) -> PredictionResult<f64> {
    if !delta.is_finite() {
        return Err(PredictionError::NonFinite);
    }
    if delta <= 0.0 {
        return Err(PredictionError::InvalidAmount {
            reason: "share amount must be positive".into(),
        });
    }
    if outcome_index >= q.len() {
        return Err(PredictionError::InvalidOutcome {
            outcome_index,
            num_outcomes: q.len(),
        });
    }
    let c_before = cost(q, b)?;
    let mut q_after = q.to_vec();
    q_after[outcome_index] += delta;
    let c_after = cost(&q_after, b)?;
    Ok(c_after - c_before)
}

/// Compute the proceeds from selling `delta` shares of outcome `i`.
///
/// Returns the net WRF proceeds (in f64 base units) the trader receives:
///   proceeds = C(q) − C(q')
/// where q'[i] = q[i] − delta and q'[j] = q[j] for j ≠ i.
pub fn sell_proceeds(
    q: &[f64],
    b: f64,
    outcome_index: usize,
    delta: f64,
) -> PredictionResult<f64> {
    if !delta.is_finite() {
        return Err(PredictionError::NonFinite);
    }
    if delta <= 0.0 {
        return Err(PredictionError::InvalidAmount {
            reason: "share amount must be positive".into(),
        });
    }
    if outcome_index >= q.len() {
        return Err(PredictionError::InvalidOutcome {
            outcome_index,
            num_outcomes: q.len(),
        });
    }
    if q[outcome_index] < delta {
        return Err(PredictionError::InsufficientFunds {
            required: delta as u128,
            available: q[outcome_index] as u128,
        });
    }
    let c_before = cost(q, b)?;
    let mut q_after = q.to_vec();
    q_after[outcome_index] -= delta;
    let c_after = cost(&q_after, b)?;
    // proceeds = decrease in cost function
    Ok(c_before - c_after)
}

/// Implied probability of outcome `i` under LMSR:
///   p(i) = exp(qᵢ/b) / Σⱼ exp(qⱼ/b)
///
/// This is the first derivative of C with respect to qᵢ.
pub fn price(q: &[f64], b: f64, outcome_index: usize) -> PredictionResult<f64> {
    if !b.is_finite() {
        return Err(PredictionError::NonFinite);
    }
    for &qi in q {
        if !qi.is_finite() {
            return Err(PredictionError::NonFinite);
        }
    }
    if outcome_index >= q.len() {
        return Err(PredictionError::InvalidOutcome {
            outcome_index,
            num_outcomes: q.len(),
        });
    }
    // Numerical stability: subtract max before exp.
    let max_val = q.iter().cloned().fold(f64::NEG_INFINITY, f64::max) / b;
    let numerator = (q[outcome_index] / b - max_val).exp();
    let denominator: f64 = q.iter().map(|&qi| (qi / b - max_val).exp()).sum();
    Ok(numerator / denominator)
}

/// Convert an f64 LMSR cost value to integer WRF base units, rounding up
/// so the protocol never under-charges (at most 1 base unit rounding loss).
pub fn lmsr_cost_to_base_units(cost_f64: f64) -> u128 {
    cost_f64.ceil() as u128
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cost_symmetric_initial() {
        // At q=[0,0], C = b*ln(2).
        let b = 100.0;
        let q = vec![0.0, 0.0];
        let c = cost(&q, b).unwrap();
        let expected = b * 2.0_f64.ln();
        assert!((c - expected).abs() < 1e-9, "C([0,0]) = b·ln(2)");
    }

    #[test]
    fn test_cost_monotone() {
        // Buying shares must increase cost.
        let b = 100.0;
        let mut q = vec![0.0, 0.0];
        let c0 = cost(&q, b).unwrap();
        q[0] += 10.0;
        let c1 = cost(&q, b).unwrap();
        q[0] += 10.0;
        let c2 = cost(&q, b).unwrap();
        assert!(c1 > c0, "cost should increase after buying shares");
        assert!(c2 > c1, "cost should keep increasing");
    }

    #[test]
    fn test_buy_cost_positive() {
        let b = 100.0;
        let q = vec![0.0, 0.0];
        let cost = buy_cost(&q, b, 0, 5.0).unwrap();
        assert!(cost > 0.0, "buying shares has positive cost");
    }

    #[test]
    fn test_sell_proceeds_positive() {
        let b = 100.0;
        let q = vec![10.0, 0.0];
        let proceeds = sell_proceeds(&q, b, 0, 5.0).unwrap();
        assert!(proceeds > 0.0, "selling shares yields positive proceeds");
    }

    #[test]
    fn test_price_sums_to_one() {
        let b = 100.0;
        let q = vec![10.0, 5.0, 2.0];
        let total: f64 = (0..3).map(|i| price(&q, b, i).unwrap()).sum();
        assert!((total - 1.0).abs() < 1e-12, "prices must sum to 1");
    }

    #[test]
    fn test_price_increases_with_shares() {
        let b = 100.0;
        let q_before = vec![0.0, 0.0];
        let q_after = vec![10.0, 0.0];
        let p_before = price(&q_before, b, 0).unwrap();
        let p_after = price(&q_after, b, 0).unwrap();
        assert!(p_after > p_before, "buying Yes increases Yes price");
        // No price should decrease correspondingly.
        let p_no_before = price(&q_before, b, 1).unwrap();
        let p_no_after = price(&q_after, b, 1).unwrap();
        assert!(p_no_after < p_no_before, "buying Yes decreases No price");
    }

    #[test]
    fn test_invalid_b() {
        assert!(cost(&[0.0, 0.0], 0.0).is_err());
        assert!(cost(&[0.0, 0.0], -1.0).is_err());
    }

    #[test]
    fn test_invalid_outcome_index() {
        let q = vec![0.0, 0.0];
        assert!(buy_cost(&q, 100.0, 5, 1.0).is_err());
        assert!(sell_proceeds(&q, 100.0, 5, 1.0).is_err());
        assert!(price(&q, 100.0, 5).is_err());
    }

    #[test]
    fn test_buy_then_sell_roundtrip() {
        // Selling immediately after buying should nearly return what was spent.
        let b = 100.0;
        let q = vec![0.0, 0.0];
        let bought = buy_cost(&q, b, 0, 5.0).unwrap();
        let mut q2 = q.clone();
        q2[0] += 5.0;
        let sold = sell_proceeds(&q2, b, 0, 5.0).unwrap();
        assert!((bought - sold).abs() < 1e-9, "buy then sell roundtrip: diff={}", bought - sold);
    }

    #[test]
    fn test_lmsr_cost_to_base_units_rounds_up() {
        assert_eq!(lmsr_cost_to_base_units(1.0), 1);
        assert_eq!(lmsr_cost_to_base_units(1.1), 2);
        assert_eq!(lmsr_cost_to_base_units(1.9999), 2);
        assert_eq!(lmsr_cost_to_base_units(100.0), 100);
    }

    #[test]
    fn test_nan_b_rejected() {
        assert!(matches!(cost(&[0.0, 0.0], f64::NAN), Err(PredictionError::NonFinite)));
    }

    #[test]
    fn test_inf_b_rejected() {
        assert!(matches!(cost(&[0.0, 0.0], f64::INFINITY), Err(PredictionError::NonFinite)));
    }

    #[test]
    fn test_nan_q_rejected() {
        assert!(matches!(cost(&[f64::NAN, 0.0], 100.0), Err(PredictionError::NonFinite)));
    }

    #[test]
    fn test_inf_delta_buy_rejected() {
        assert!(matches!(buy_cost(&[0.0, 0.0], 100.0, 0, f64::INFINITY), Err(PredictionError::NonFinite)));
    }

    #[test]
    fn test_nan_delta_sell_rejected() {
        assert!(matches!(sell_proceeds(&[10.0, 0.0], 100.0, 0, f64::NAN), Err(PredictionError::NonFinite)));
    }
}
