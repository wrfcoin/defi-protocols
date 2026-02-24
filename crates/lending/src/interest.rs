use serde::{Deserialize, Serialize};

/// Trait for pluggable interest rate models.
pub trait InterestRateModel {
    /// Calculate the borrow rate given current utilization (in basis points).
    /// Returns the annualized borrow rate in basis points.
    fn calculate_borrow_rate(&self, utilization_bps: u64) -> u64;

    /// Calculate the supply rate from the borrow rate and utilization.
    /// supply_rate = borrow_rate * utilization / 10_000
    fn calculate_supply_rate(&self, utilization_bps: u64) -> u64 {
        let borrow_rate = self.calculate_borrow_rate(utilization_bps) as u128;
        let util = utilization_bps as u128;
        ((borrow_rate * util) / 10_000) as u64
    }
}

/// A two-slope linear interest rate model (similar to Aave/Compound).
///
/// Below optimal utilization: rate = base_rate + (utilization / optimal) * slope1
/// Above optimal utilization: rate = base_rate + slope1 + ((utilization - optimal) / (1 - optimal)) * slope2
///
/// All parameters are in basis points.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinearInterestRate {
    /// Base borrow rate when utilization is 0 (in bps).
    pub base_rate_bps: u64,
    /// Slope for utilization below the optimal point (in bps).
    pub slope1_bps: u64,
    /// Slope for utilization above the optimal point (in bps).
    pub slope2_bps: u64,
    /// Optimal utilization target (in bps, e.g., 8000 = 80%).
    pub optimal_utilization_bps: u64,
}

impl LinearInterestRate {
    pub fn new(
        base_rate_bps: u64,
        slope1_bps: u64,
        slope2_bps: u64,
        optimal_utilization_bps: u64,
    ) -> Self {
        Self {
            base_rate_bps,
            slope1_bps,
            slope2_bps,
            optimal_utilization_bps,
        }
    }
}

impl InterestRateModel for LinearInterestRate {
    fn calculate_borrow_rate(&self, utilization_bps: u64) -> u64 {
        let base = self.base_rate_bps as u128;
        let util = utilization_bps as u128;
        let optimal = self.optimal_utilization_bps as u128;
        let slope1 = self.slope1_bps as u128;
        let slope2 = self.slope2_bps as u128;

        if optimal == 0 {
            // Edge case: everything is above optimal.
            return (base + slope1 + slope2) as u64;
        }

        if util <= optimal {
            // Below kink: base + utilization * slope1 / optimal
            let variable = (util * slope1) / optimal;
            (base + variable) as u64
        } else {
            // Above kink: base + slope1 + excess * slope2 / (10_000 - optimal)
            let excess = util - optimal;
            let denominator = 10_000u128 - optimal;
            if denominator == 0 {
                return (base + slope1 + slope2) as u64;
            }
            let variable = (excess * slope2) / denominator;
            (base + slope1 + variable) as u64
        }
    }
}

impl Default for LinearInterestRate {
    fn default() -> Self {
        // Sensible defaults: 2% base, 4% slope1, 75% slope2, 80% optimal utilization
        Self::new(200, 400, 7500, 8000)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_values() {
        let model = LinearInterestRate::default();
        assert_eq!(model.base_rate_bps, 200);
        assert_eq!(model.slope1_bps, 400);
        assert_eq!(model.slope2_bps, 7500);
        assert_eq!(model.optimal_utilization_bps, 8000);
    }

    #[test]
    fn test_zero_utilization() {
        let model = LinearInterestRate::default();
        // At 0% utilization: rate = base = 200 bps (2%)
        assert_eq!(model.calculate_borrow_rate(0), 200);
    }

    #[test]
    fn test_at_optimal_utilization() {
        let model = LinearInterestRate::default();
        // At 80% (optimal): rate = base + slope1 = 200 + 400 = 600 bps (6%)
        assert_eq!(model.calculate_borrow_rate(8000), 600);
    }

    #[test]
    fn test_below_optimal() {
        let model = LinearInterestRate::default();
        // At 40% utilization: rate = 200 + (4000 * 400 / 8000) = 200 + 200 = 400 bps
        assert_eq!(model.calculate_borrow_rate(4000), 400);
    }

    #[test]
    fn test_above_optimal() {
        let model = LinearInterestRate::default();
        // At 90% utilization: excess = 1000, denominator = 2000
        // rate = 200 + 400 + (1000 * 7500 / 2000) = 600 + 3750 = 4350 bps
        assert_eq!(model.calculate_borrow_rate(9000), 4350);
    }

    #[test]
    fn test_full_utilization() {
        let model = LinearInterestRate::default();
        // At 100%: excess = 2000, denominator = 2000
        // rate = 200 + 400 + 7500 = 8100 bps (81%)
        assert_eq!(model.calculate_borrow_rate(10_000), 8100);
    }

    #[test]
    fn test_optimal_zero() {
        let model = LinearInterestRate::new(100, 200, 300, 0);
        // Everything above optimal
        assert_eq!(model.calculate_borrow_rate(5000), 100 + 200 + 300);
    }

    #[test]
    fn test_supply_rate() {
        let model = LinearInterestRate::default();
        // supply = borrow_rate * util / 10_000
        let borrow_rate = model.calculate_borrow_rate(8000);
        let supply_rate = model.calculate_supply_rate(8000);
        // 600 * 8000 / 10_000 = 480
        assert_eq!(supply_rate, (borrow_rate as u128 * 8000 / 10_000) as u64);
    }
}
