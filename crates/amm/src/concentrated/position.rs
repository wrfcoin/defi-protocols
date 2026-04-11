use wrfcoin_defi_common::types::Address;

/// Unique identifier for a concentrated liquidity position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PositionId(pub u64);

impl PositionId {
    pub fn new(id: u64) -> Self {
        Self(id)
    }
}

/// A concentrated liquidity LP position tracking a specific tick range and
/// the LP's liquidity and fee growth checkpoints.
#[derive(Debug, Clone)]
pub struct ConcentratedPosition {
    /// Owner of this position.
    pub owner: Address,
    /// Lower tick bound of the range (inclusive).
    pub lower_tick: i32,
    /// Upper tick bound of the range (exclusive).
    pub upper_tick: i32,
    /// Amount of virtual liquidity provided in this range.
    pub liquidity: u128,
    /// Fee growth of token0 *inside* the range at the last time fees were
    /// collected for this position. Used to compute accrued fees.
    pub fee_growth_inside_last_0: u128,
    /// Fee growth of token1 *inside* the range at the last time fees were
    /// collected for this position.
    pub fee_growth_inside_last_1: u128,
    /// Uncollected fees in token0 units (accumulated, pending withdrawal).
    pub tokens_owed_0: u128,
    /// Uncollected fees in token1 units.
    pub tokens_owed_1: u128,
}

impl ConcentratedPosition {
    /// Create a new empty position.
    pub fn new(owner: Address, lower_tick: i32, upper_tick: i32) -> Self {
        Self {
            owner,
            lower_tick,
            upper_tick,
            liquidity: 0,
            fee_growth_inside_last_0: 0,
            fee_growth_inside_last_1: 0,
            tokens_owed_0: 0,
            tokens_owed_1: 0,
        }
    }

    /// Update the position when liquidity is added or removed.
    ///
    /// Accrues any fees earned since the last checkpoint before modifying liquidity.
    ///
    /// `fee_growth_inside_0/1` are the *current* fee growth inside this tick range.
    /// `liquidity_delta`: positive to add liquidity, negative to remove.
    ///
    /// Returns `Err` if removing more liquidity than available.
    pub fn update(
        &mut self,
        liquidity_delta: i128,
        fee_growth_inside_0: u128,
        fee_growth_inside_1: u128,
    ) -> Result<(), &'static str> {
        // Accrue fees before changing liquidity.
        if self.liquidity > 0 {
            let accrued_0 = fee_growth_inside_0
                .wrapping_sub(self.fee_growth_inside_last_0)
                .saturating_mul(self.liquidity)
                >> 64; // scale from Q64.64 per-liquidity to base units
            let accrued_1 = fee_growth_inside_1
                .wrapping_sub(self.fee_growth_inside_last_1)
                .saturating_mul(self.liquidity)
                >> 64;
            self.tokens_owed_0 = self.tokens_owed_0.saturating_add(accrued_0);
            self.tokens_owed_1 = self.tokens_owed_1.saturating_add(accrued_1);
        }

        // Update fee growth checkpoints.
        self.fee_growth_inside_last_0 = fee_growth_inside_0;
        self.fee_growth_inside_last_1 = fee_growth_inside_1;

        // Apply liquidity delta.
        if liquidity_delta >= 0 {
            self.liquidity = self
                .liquidity
                .checked_add(liquidity_delta as u128)
                .ok_or("liquidity overflow")?;
        } else {
            let remove = (-liquidity_delta) as u128;
            if remove > self.liquidity {
                return Err("insufficient liquidity to burn");
            }
            self.liquidity -= remove;
        }

        Ok(())
    }

    /// Collect accrued fees. Returns (tokens_owed_0, tokens_owed_1) and resets
    /// the internal counters to zero.
    pub fn collect_fees(&mut self) -> (u128, u128) {
        let owed = (self.tokens_owed_0, self.tokens_owed_1);
        self.tokens_owed_0 = 0;
        self.tokens_owed_1 = 0;
        owed
    }

    /// Check if this position is "active" (has non-zero liquidity).
    pub fn is_active(&self) -> bool {
        self.liquidity > 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owner() -> Address {
        Address::from_bytes([1u8; 32])
    }

    #[test]
    fn test_position_new() {
        let pos = ConcentratedPosition::new(owner(), -10, 10);
        assert_eq!(pos.lower_tick, -10);
        assert_eq!(pos.upper_tick, 10);
        assert_eq!(pos.liquidity, 0);
        assert!(!pos.is_active());
    }

    #[test]
    fn test_position_add_liquidity() {
        let mut pos = ConcentratedPosition::new(owner(), 0, 100);
        pos.update(1_000_000, 0, 0).unwrap();
        assert_eq!(pos.liquidity, 1_000_000);
        assert!(pos.is_active());
    }

    #[test]
    fn test_position_remove_liquidity() {
        let mut pos = ConcentratedPosition::new(owner(), 0, 100);
        pos.update(1_000_000, 0, 0).unwrap();
        pos.update(-500_000, 0, 0).unwrap();
        assert_eq!(pos.liquidity, 500_000);
    }

    #[test]
    fn test_position_remove_too_much_fails() {
        let mut pos = ConcentratedPosition::new(owner(), 0, 100);
        pos.update(100, 0, 0).unwrap();
        let result = pos.update(-200, 0, 0);
        assert!(result.is_err());
    }

    #[test]
    fn test_position_fee_accrual() {
        let mut pos = ConcentratedPosition::new(owner(), 0, 100);
        // Add liquidity with initial fee growth = 0
        pos.update(1_000_000, 0, 0).unwrap();
        // Simulate fee growth inside the range: fee_growth_inside increased by 2^64 units
        // (i.e., 1 token per unit of liquidity in Q64.64 terms)
        let fee_growth_delta: u128 = 1u128 << 64; // 1.0 in Q64.64
        pos.update(0, fee_growth_delta, fee_growth_delta * 2).unwrap();
        // accrued_0 = (fee_growth_delta - 0) * liquidity >> 64
        //           = (2^64) * 1_000_000 >> 64 = 1_000_000
        // accrued_1 = (2 * 2^64) * 1_000_000 >> 64 = 2_000_000
        assert_eq!(pos.tokens_owed_0, 1_000_000);
        assert_eq!(pos.tokens_owed_1, 2_000_000);
    }

    #[test]
    fn test_position_collect_fees() {
        let mut pos = ConcentratedPosition::new(owner(), 0, 100);
        pos.update(1_000_000, 0, 0).unwrap();
        let fee_growth: u128 = 1u128 << 64;
        pos.update(0, fee_growth, fee_growth).unwrap();
        let (f0, f1) = pos.collect_fees();
        assert_eq!(f0, 1_000_000);
        assert_eq!(f1, 1_000_000);
        // After collect, owed is zero
        let (f0, f1) = pos.collect_fees();
        assert_eq!(f0, 0);
        assert_eq!(f1, 0);
    }
}
