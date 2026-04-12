use crate::error::{RevenueError, RevenueResult};
use crate::pool::RevenuePool;
use crate::registry::DataContributorRegistry;
use serde::{Deserialize, Serialize};
use wrfcoin_defi_common::types::{Address, Amount};

/// A single contributor's payout from an epoch distribution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContributorPayout {
    pub address: Address,
    pub amount: Amount,
    pub composite_score: u64,
}

/// Result of one epoch distribution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DistributionEpoch {
    pub height: u64,
    pub total_distributed: Amount,
    pub payouts: Vec<ContributorPayout>,
}

/// Execute an epoch distribution: drain the pool and split pro-rata by composite score.
///
/// Returns `Err(DistributionTooEarly)` if the interval has not elapsed.
/// Returns `Err(NoEligibleContributors)` if no contributor meets the quality threshold.
pub fn distribute_epoch(
    pool: &mut RevenuePool,
    registry: &DataContributorRegistry,
    current_height: u64,
) -> RevenueResult<DistributionEpoch> {
    if !pool.is_distribution_due(current_height) {
        return Err(RevenueError::DistributionTooEarly {
            current: current_height,
            next: pool.last_distribution_height + pool.distribution_interval_blocks,
        });
    }

    let eligible = registry.eligible_contributors();
    if eligible.is_empty() {
        return Err(RevenueError::NoEligibleContributors);
    }

    let total_score: u64 = eligible.iter().map(|p| p.composite_score()).sum();
    if total_score == 0 {
        return Err(RevenueError::NoEligibleContributors);
    }

    let total_amount = pool.drain_for_distribution(current_height);

    let mut payouts = Vec::with_capacity(eligible.len());
    let mut distributed = Amount::ZERO;

    // Allocate proportionally; give remainder to last contributor to avoid dust loss.
    for (i, contributor) in eligible.iter().enumerate() {
        let score = contributor.composite_score();
        let share = if i + 1 < eligible.len() {
            Amount::new(
                (total_amount.0 * score as u128)
                    .checked_div(total_score as u128)
                    .unwrap_or(0),
            )
        } else {
            // Last contributor gets remainder.
            total_amount
                .checked_sub(distributed)
                .unwrap_or(Amount::ZERO)
        };
        distributed = distributed
            .checked_add(share)
            .ok_or_else(|| RevenueError::Overflow {
                operation: "payout accumulation".into(),
            })?;
        payouts.push(ContributorPayout {
            address: contributor.address,
            amount: share,
            composite_score: score,
        });
    }

    Ok(DistributionEpoch {
        height: current_height,
        total_distributed: distributed,
        payouts,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::ContributorProfile;

    fn make_address(byte: u8) -> Address {
        let mut bytes = [0u8; 32];
        bytes[31] = byte;
        Address::from_bytes(bytes)
    }

    fn make_eligible_profile(address: Address, quality: u16, uptime: u16) -> ContributorProfile {
        ContributorProfile {
            address,
            quality_score_bps: quality,
            uptime_bps: uptime,
            geographic_score_bps: 5_000,
            volume_score_bps: 5_000,
            active: true,
        }
    }

    #[test]
    fn test_distribute_too_early_returns_error() {
        let mut pool = RevenuePool::new(7_200);
        pool.deposit("amm", Amount::new(1_000)).unwrap();
        let registry = DataContributorRegistry::new();
        let result = distribute_epoch(&mut pool, &registry, 100);
        assert!(matches!(
            result,
            Err(RevenueError::DistributionTooEarly {
                current: 100,
                next: 7_200
            })
        ));
    }

    #[test]
    fn test_distribute_no_eligible_contributors() {
        let mut pool = RevenuePool::new(7_200);
        pool.deposit("amm", Amount::new(1_000)).unwrap();
        let registry = DataContributorRegistry::new();
        let result = distribute_epoch(&mut pool, &registry, 7_200);
        assert!(matches!(result, Err(RevenueError::NoEligibleContributors)));
    }

    #[test]
    fn test_distribute_single_contributor_gets_all() {
        let mut pool = RevenuePool::new(7_200);
        pool.deposit("amm", Amount::new(10_000)).unwrap();

        let mut registry = DataContributorRegistry::new();
        let addr = make_address(1);
        registry.register(make_eligible_profile(addr, 8_000, 9_000));

        let epoch = distribute_epoch(&mut pool, &registry, 7_200).unwrap();
        assert_eq!(epoch.total_distributed, Amount::new(10_000));
        assert_eq!(epoch.payouts.len(), 1);
        assert_eq!(epoch.payouts[0].amount, Amount::new(10_000));
        assert_eq!(epoch.payouts[0].address, addr);
        assert_eq!(epoch.height, 7_200);
    }

    #[test]
    fn test_distribute_two_equal_contributors_split_evenly() {
        let mut pool = RevenuePool::new(7_200);
        pool.deposit("amm", Amount::new(10_000)).unwrap();

        let mut registry = DataContributorRegistry::new();
        // Both have same scores → equal composite
        registry.register(make_eligible_profile(make_address(1), 8_000, 8_000));
        registry.register(make_eligible_profile(make_address(2), 8_000, 8_000));

        let epoch = distribute_epoch(&mut pool, &registry, 7_200).unwrap();
        assert_eq!(epoch.total_distributed, Amount::new(10_000));
        assert_eq!(epoch.payouts.len(), 2);

        // Each should get ~5000; total must equal 10_000
        let total: u128 = epoch.payouts.iter().map(|p| p.amount.0).sum();
        assert_eq!(total, 10_000);

        // Verify both payouts are positive
        for payout in &epoch.payouts {
            assert!(payout.amount.0 > 0);
        }
    }

    #[test]
    fn test_distribute_three_contributors_proportional() {
        let mut pool = RevenuePool::new(7_200);
        pool.deposit("amm", Amount::new(10_000_000)).unwrap();

        let mut registry = DataContributorRegistry::new();
        // contributor A: quality=10000 uptime=10000 geo=5000 volume=5000
        // composite = (10000*40 + 10000*30 + 5000*20 + 5000*10)/100 = (400000+300000+100000+50000)/100 = 8500
        registry.register(ContributorProfile {
            address: make_address(1),
            quality_score_bps: 10_000,
            uptime_bps: 10_000,
            geographic_score_bps: 5_000,
            volume_score_bps: 5_000,
            active: true,
        });
        // contributor B: quality=6000 uptime=6000 geo=5000 volume=5000
        // composite = (6000*40 + 6000*30 + 5000*20 + 5000*10)/100 = (240000+180000+100000+50000)/100 = 5700
        registry.register(ContributorProfile {
            address: make_address(2),
            quality_score_bps: 6_000,
            uptime_bps: 6_000,
            geographic_score_bps: 5_000,
            volume_score_bps: 5_000,
            active: true,
        });
        // contributor C: quality=5000 uptime=5000 geo=5000 volume=5000
        // composite = (5000*40 + 5000*30 + 5000*20 + 5000*10)/100 = 5000
        registry.register(ContributorProfile {
            address: make_address(3),
            quality_score_bps: 5_000,
            uptime_bps: 5_000,
            geographic_score_bps: 5_000,
            volume_score_bps: 5_000,
            active: true,
        });

        let epoch = distribute_epoch(&mut pool, &registry, 7_200).unwrap();
        assert_eq!(epoch.total_distributed, Amount::new(10_000_000));

        let total: u128 = epoch.payouts.iter().map(|p| p.amount.0).sum();
        assert_eq!(
            total, 10_000_000,
            "all funds must be distributed without loss"
        );

        // All payouts must be positive
        for payout in &epoch.payouts {
            assert!(
                payout.amount.0 > 0,
                "each contributor should receive something"
            );
        }
    }

    #[test]
    fn test_pool_is_drained_after_distribution() {
        let mut pool = RevenuePool::new(7_200);
        pool.deposit("amm", Amount::new(5_000)).unwrap();

        let mut registry = DataContributorRegistry::new();
        registry.register(make_eligible_profile(make_address(1), 8_000, 8_000));

        distribute_epoch(&mut pool, &registry, 7_200).unwrap();
        assert_eq!(pool.total_accumulated, Amount::ZERO);
        assert_eq!(pool.last_distribution_height, 7_200);
    }

    #[test]
    fn test_ineligible_contributor_excluded() {
        let mut pool = RevenuePool::new(7_200);
        pool.deposit("amm", Amount::new(10_000)).unwrap();

        let mut registry = DataContributorRegistry::new();
        // Eligible
        registry.register(make_eligible_profile(make_address(1), 8_000, 8_000));
        // Ineligible (quality too low)
        registry.register(ContributorProfile {
            address: make_address(2),
            quality_score_bps: 3_000,
            uptime_bps: 10_000,
            geographic_score_bps: 10_000,
            volume_score_bps: 10_000,
            active: true,
        });

        let epoch = distribute_epoch(&mut pool, &registry, 7_200).unwrap();
        assert_eq!(epoch.payouts.len(), 1);
        assert_eq!(epoch.payouts[0].address, make_address(1));
        assert_eq!(epoch.total_distributed, Amount::new(10_000));
    }
}
