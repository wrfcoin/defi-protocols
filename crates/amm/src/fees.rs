use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use wrfcoin_defi_common::error::{DefiError, DefiResult};
use wrfcoin_defi_common::types::{Amount, PoolId};

/// Predefined fee tiers in basis points.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FeeTier {
    /// 0.01% — for stable pairs.
    OneBasicPoint,
    /// 0.05% — for highly correlated pairs.
    FiveBasicPoints,
    /// 0.3% — standard tier for most pairs.
    ThirtyBasicPoints,
    /// 1% — for exotic/volatile pairs.
    OnePercent,
}

impl FeeTier {
    /// Return the fee rate in basis points.
    pub fn as_bps(&self) -> u64 {
        match self {
            FeeTier::OneBasicPoint => 1,
            FeeTier::FiveBasicPoints => 5,
            FeeTier::ThirtyBasicPoints => 30,
            FeeTier::OnePercent => 100,
        }
    }
}

/// Breakdown of how a fee is split.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct FeeDistribution {
    /// Amount going to liquidity providers.
    pub lp_fee: Amount,
    /// Amount going to the protocol treasury.
    pub protocol_fee: Amount,
}

/// Manages fee calculations and distribution for AMM pools.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeeManager {
    /// Fee tier assigned to each pool.
    pub fee_tiers: HashMap<PoolId, FeeTier>,
    /// Protocol fee share in basis points of the total swap fee (e.g., 1667 = 1/6 of 30bps).
    pub protocol_fee_share_bps: u64,
    /// Accumulated protocol fees per pool.
    pub accumulated_protocol_fees: HashMap<PoolId, Amount>,
}

impl FeeManager {
    pub fn new(protocol_fee_share_bps: u64) -> Self {
        Self {
            fee_tiers: HashMap::new(),
            protocol_fee_share_bps,
            accumulated_protocol_fees: HashMap::new(),
        }
    }

    /// Register a fee tier for a pool.
    pub fn set_fee_tier(&mut self, pool_id: PoolId, tier: FeeTier) {
        self.fee_tiers.insert(pool_id, tier);
    }

    /// Get the fee rate in basis points for a pool.
    pub fn get_fee_rate_bps(&self, pool_id: &PoolId) -> u64 {
        self.fee_tiers
            .get(pool_id)
            .map(|t| t.as_bps())
            .unwrap_or(FeeTier::ThirtyBasicPoints.as_bps())
    }

    /// Calculate the total fee for a swap amount and return the fee breakdown.
    pub fn calculate_fee(
        &self,
        pool_id: &PoolId,
        amount_in: Amount,
    ) -> DefiResult<FeeDistribution> {
        let fee_bps = self.get_fee_rate_bps(pool_id) as u128;

        // Total fee = amount_in * fee_bps / 10_000
        let total_fee = amount_in
            .mul_fraction(fee_bps, 10_000)
            .ok_or(DefiError::Overflow {
                operation: "calculate_fee total".into(),
            })?;

        // Protocol fee = total_fee * protocol_share / 10_000
        let protocol_fee = total_fee
            .mul_fraction(self.protocol_fee_share_bps as u128, 10_000)
            .ok_or(DefiError::Overflow {
                operation: "calculate_fee protocol".into(),
            })?;

        // LP fee = total - protocol
        let lp_fee = total_fee
            .checked_sub(protocol_fee)
            .ok_or(DefiError::Overflow {
                operation: "calculate_fee lp".into(),
            })?;

        Ok(FeeDistribution {
            lp_fee,
            protocol_fee,
        })
    }

    /// Record fee distribution: accumulate the protocol's share.
    pub fn distribute_fees(
        &mut self,
        pool_id: &PoolId,
        distribution: &FeeDistribution,
    ) -> DefiResult<()> {
        let accumulated = self
            .accumulated_protocol_fees
            .entry(*pool_id)
            .or_insert(Amount::ZERO);
        *accumulated =
            accumulated
                .checked_add(distribution.protocol_fee)
                .ok_or(DefiError::Overflow {
                    operation: "distribute_fees accumulation".into(),
                })?;
        Ok(())
    }

    /// Withdraw accumulated protocol fees for a pool. Returns the withdrawn amount and resets to zero.
    pub fn withdraw_protocol_fees(&mut self, pool_id: &PoolId) -> Amount {
        self.accumulated_protocol_fees
            .insert(*pool_id, Amount::ZERO)
            .unwrap_or(Amount::ZERO)
    }
}

impl Default for FeeManager {
    fn default() -> Self {
        // Default: protocol takes 1/6 of the swap fee (like Uniswap v2).
        Self::new(1667)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fee_tier_bps() {
        assert_eq!(FeeTier::OneBasicPoint.as_bps(), 1);
        assert_eq!(FeeTier::FiveBasicPoints.as_bps(), 5);
        assert_eq!(FeeTier::ThirtyBasicPoints.as_bps(), 30);
        assert_eq!(FeeTier::OnePercent.as_bps(), 100);
    }

    #[test]
    fn test_default_fee_rate() {
        let fm = FeeManager::default();
        let pid = PoolId::new(99);
        // Unregistered pool gets default 30 bps
        assert_eq!(fm.get_fee_rate_bps(&pid), 30);
    }

    #[test]
    fn test_set_fee_tier() {
        let mut fm = FeeManager::default();
        let pid = PoolId::new(1);
        fm.set_fee_tier(pid, FeeTier::OnePercent);
        assert_eq!(fm.get_fee_rate_bps(&pid), 100);
    }

    #[test]
    fn test_calculate_fee() {
        let fm = FeeManager::default(); // protocol share = 1667 bps (16.67%)
        let pid = PoolId::new(1);
        // 30 bps default fee, 1_000_000 input
        let dist = fm.calculate_fee(&pid, Amount::new(1_000_000)).unwrap();
        // total = 1_000_000 * 30 / 10_000 = 3_000
        // protocol = 3_000 * 1667 / 10_000 = 500
        // lp = 3_000 - 500 = 2_500
        assert_eq!(dist.protocol_fee, Amount::new(500));
        assert_eq!(dist.lp_fee, Amount::new(2500));
    }

    #[test]
    fn test_distribute_and_withdraw_fees() {
        let mut fm = FeeManager::default();
        let pid = PoolId::new(1);
        let dist = FeeDistribution {
            lp_fee: Amount::new(100),
            protocol_fee: Amount::new(50),
        };
        fm.distribute_fees(&pid, &dist).unwrap();
        fm.distribute_fees(&pid, &dist).unwrap();
        let withdrawn = fm.withdraw_protocol_fees(&pid);
        assert_eq!(withdrawn, Amount::new(100));
        // After withdrawal, should be zero
        assert_eq!(fm.withdraw_protocol_fees(&pid), Amount::ZERO);
    }
}
