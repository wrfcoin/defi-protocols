use wrfcoin_defi_common::types::Amount;
use crate::error::{RevenueError, RevenueResult};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Accumulates protocol fees from all DeFi sources between distribution epochs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RevenuePool {
    /// Total fees accumulated across all sources since last distribution.
    pub total_accumulated: Amount,
    /// Block height of the last distribution.
    pub last_distribution_height: u64,
    /// How many blocks between epochs.
    pub distribution_interval_blocks: u64,
    /// Per-source breakdown (for auditing).
    pub source_contributions: HashMap<String, Amount>,
}

impl RevenuePool {
    pub const DEFAULT_INTERVAL_BLOCKS: u64 = 7_200; // ~1 day at 12s blocks

    pub fn new(distribution_interval_blocks: u64) -> Self {
        Self {
            total_accumulated: Amount::ZERO,
            last_distribution_height: 0,
            distribution_interval_blocks,
            source_contributions: HashMap::new(),
        }
    }

    /// Deposit fees from a named source (e.g. "amm", "lending", "yield-farming").
    pub fn deposit(&mut self, source: &str, amount: Amount) -> RevenueResult<()> {
        self.total_accumulated = self
            .total_accumulated
            .checked_add(amount)
            .ok_or(RevenueError::Overflow {
                operation: "deposit".into(),
            })?;
        let entry = self
            .source_contributions
            .entry(source.to_string())
            .or_insert(Amount::ZERO);
        *entry = entry
            .checked_add(amount)
            .ok_or(RevenueError::Overflow {
                operation: "deposit source".into(),
            })?;
        Ok(())
    }

    /// Returns true if enough blocks have passed to trigger an epoch distribution.
    pub fn is_distribution_due(&self, current_height: u64) -> bool {
        current_height >= self.last_distribution_height + self.distribution_interval_blocks
    }

    /// Drain the pool for distribution and record the block height.
    pub fn drain_for_distribution(&mut self, current_height: u64) -> Amount {
        let amount = self.total_accumulated;
        self.total_accumulated = Amount::ZERO;
        self.source_contributions.clear();
        self.last_distribution_height = current_height;
        amount
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deposit_accumulates_total() {
        let mut pool = RevenuePool::new(7_200);
        pool.deposit("amm", Amount::new(500)).unwrap();
        pool.deposit("lending", Amount::new(300)).unwrap();
        assert_eq!(pool.total_accumulated, Amount::new(800));
    }

    #[test]
    fn test_deposit_tracks_per_source() {
        let mut pool = RevenuePool::new(7_200);
        pool.deposit("amm", Amount::new(100)).unwrap();
        pool.deposit("amm", Amount::new(50)).unwrap();
        pool.deposit("lending", Amount::new(200)).unwrap();
        assert_eq!(pool.source_contributions["amm"], Amount::new(150));
        assert_eq!(pool.source_contributions["lending"], Amount::new(200));
    }

    #[test]
    fn test_is_distribution_due_false_before_interval() {
        let pool = RevenuePool::new(7_200);
        assert!(!pool.is_distribution_due(7_199));
    }

    #[test]
    fn test_is_distribution_due_true_at_interval() {
        let pool = RevenuePool::new(7_200);
        assert!(pool.is_distribution_due(7_200));
    }

    #[test]
    fn test_is_distribution_due_true_after_interval() {
        let pool = RevenuePool::new(7_200);
        assert!(pool.is_distribution_due(10_000));
    }

    #[test]
    fn test_drain_clears_pool_and_records_height() {
        let mut pool = RevenuePool::new(7_200);
        pool.deposit("amm", Amount::new(1_000)).unwrap();
        let drained = pool.drain_for_distribution(7_200);
        assert_eq!(drained, Amount::new(1_000));
        assert_eq!(pool.total_accumulated, Amount::ZERO);
        assert!(pool.source_contributions.is_empty());
        assert_eq!(pool.last_distribution_height, 7_200);
    }

    #[test]
    fn test_is_distribution_due_resets_after_drain() {
        let mut pool = RevenuePool::new(7_200);
        pool.drain_for_distribution(7_200);
        // Not due again until 14_400
        assert!(!pool.is_distribution_due(7_200));
        assert!(!pool.is_distribution_due(14_399));
        assert!(pool.is_distribution_due(14_400));
    }

    #[test]
    fn test_deposit_overflow_returns_error() {
        let mut pool = RevenuePool::new(7_200);
        pool.deposit("amm", Amount::new(u128::MAX)).unwrap();
        let result = pool.deposit("amm", Amount::new(1));
        assert!(matches!(result, Err(RevenueError::Overflow { .. })));
    }
}
