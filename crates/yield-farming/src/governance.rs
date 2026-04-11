use serde::{Deserialize, Serialize};
use wrfcoin_defi_common::error::{DefiError, DefiResult};
use wrfcoin_defi_common::types::Amount;

const BPS_SCALE: u32 = 10_000;

/// Configuration for governance-weighted reward routing.
///
/// Governance only rebalances a fixed emission budget across pools. It does not
/// mint extra rewards, which avoids adding another inflation layer on top of
/// the existing reward system.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct GovernanceBoostConfig {
    /// Governance score treated as neutral (1.0x routing multiplier).
    pub neutral_governance_bps: u16,
    /// Maximum routing tilt in basis points.
    ///
    /// Example: `2_000` means governance can move a pool between `0.8x` and
    /// `1.2x` of its base allocation before normalization.
    pub max_tilt_bps: u16,
}

impl Default for GovernanceBoostConfig {
    fn default() -> Self {
        Self {
            neutral_governance_bps: 5_000,
            max_tilt_bps: 2_000,
        }
    }
}

impl GovernanceBoostConfig {
    pub fn validate(&self) -> DefiResult<()> {
        if self.neutral_governance_bps == 0
            || self.neutral_governance_bps >= BPS_SCALE as u16
        {
            return Err(DefiError::InvalidAmount {
                reason: "neutral_governance_bps must be between 1 and 9_999".into(),
            });
        }

        if self.max_tilt_bps > BPS_SCALE as u16 {
            return Err(DefiError::InvalidAmount {
                reason: "max_tilt_bps must be <= 10_000".into(),
            });
        }

        Ok(())
    }
}

/// Governance input for a single pool.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct GovernancePoolInput {
    pub alloc_point: u64,
    pub governance_score_bps: u16,
}

impl GovernancePoolInput {
    pub fn new(alloc_point: u64, governance_score_bps: u16) -> Self {
        Self {
            alloc_point,
            governance_score_bps,
        }
    }
}

/// Per-pool routing result under governance normalization.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct GovernanceEmissionAllocation {
    pub alloc_point: u64,
    pub governance_score_bps: u16,
    pub governance_multiplier_bps: u16,
    pub routed_emission: Amount,
}

/// Deterministic governance router that preserves the fixed protocol budget.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
pub struct GovernanceEmissionRouter {
    pub config: GovernanceBoostConfig,
}

impl GovernanceEmissionRouter {
    pub fn with_config(config: GovernanceBoostConfig) -> DefiResult<Self> {
        config.validate()?;
        Ok(Self { config })
    }

    /// Governance multiplier centered around 1.0x at the neutral score.
    pub fn governance_multiplier_bps(&self, governance_score_bps: u16) -> u16 {
        let score = governance_score_bps.min(BPS_SCALE as u16) as i32;
        let neutral = self.config.neutral_governance_bps as i32;
        let max_tilt = self.config.max_tilt_bps as i32;
        let delta = score - neutral;
        let adjustment = (delta * max_tilt) / neutral.max(1);
        let multiplier = (BPS_SCALE as i32 + adjustment)
            .clamp(0, u16::MAX as i32);
        multiplier as u16
    }

    fn raw_weight(&self, pool: &GovernancePoolInput) -> u128 {
        u128::from(pool.alloc_point) * u128::from(self.governance_multiplier_bps(pool.governance_score_bps))
    }

    /// Route a fixed emission budget across pools using normalized governance
    /// weights. The sum of all `routed_emission` values is always equal to the
    /// input `emission_budget`.
    pub fn route_fixed_budget(
        &self,
        emission_budget: Amount,
        pools: &[GovernancePoolInput],
    ) -> DefiResult<Vec<GovernanceEmissionAllocation>> {
        self.config.validate()?;

        if pools.is_empty() {
            return Ok(Vec::new());
        }

        let total_raw_weight: u128 = pools.iter().map(|pool| self.raw_weight(pool)).sum();
        if total_raw_weight == 0 || emission_budget.is_zero() {
            return Ok(pools
                .iter()
                .map(|pool| GovernanceEmissionAllocation {
                    alloc_point: pool.alloc_point,
                    governance_score_bps: pool.governance_score_bps.min(BPS_SCALE as u16),
                    governance_multiplier_bps: self
                        .governance_multiplier_bps(pool.governance_score_bps),
                    routed_emission: Amount::ZERO,
                })
                .collect());
        }

        let mut routed_total = 0u128;
        let mut allocations: Vec<(GovernanceEmissionAllocation, u128)> = pools
            .iter()
            .map(|pool| {
                let raw_weight = self.raw_weight(pool);
                let numerator = emission_budget
                    .0
                    .checked_mul(raw_weight)
                    .ok_or(DefiError::Overflow {
                        operation: "governance routing numerator".into(),
                    })?;
                let routed = numerator / total_raw_weight;
                let remainder = numerator % total_raw_weight;
                routed_total = routed_total.checked_add(routed).ok_or(DefiError::Overflow {
                    operation: "governance routing sum".into(),
                })?;

                Ok((
                    GovernanceEmissionAllocation {
                        alloc_point: pool.alloc_point,
                        governance_score_bps: pool.governance_score_bps.min(BPS_SCALE as u16),
                        governance_multiplier_bps: self
                            .governance_multiplier_bps(pool.governance_score_bps),
                        routed_emission: Amount::new(routed),
                    },
                    remainder,
                ))
            })
            .collect::<DefiResult<Vec<_>>>()?;

        let leftover = emission_budget.0.checked_sub(routed_total).ok_or(DefiError::Overflow {
            operation: "governance routing leftover".into(),
        })?;

        let mut by_remainder: Vec<(usize, u128)> = allocations
            .iter()
            .enumerate()
            .map(|(index, (_, remainder))| (index, *remainder))
            .collect();
        by_remainder.sort_by(|(left_idx, left_remainder), (right_idx, right_remainder)| {
            right_remainder
                .cmp(left_remainder)
                .then_with(|| left_idx.cmp(right_idx))
        });

        for (index, _) in by_remainder.into_iter().take(leftover as usize) {
            allocations[index].0.routed_emission = allocations[index]
                .0
                .routed_emission
                .checked_add(Amount::new(1))
                .ok_or(DefiError::Overflow {
                    operation: "governance routing remainder distribution".into(),
                })?;
        }

        Ok(allocations.into_iter().map(|(allocation, _)| allocation).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sum_emissions(allocations: &[GovernanceEmissionAllocation]) -> Amount {
        allocations
            .iter()
            .fold(Amount::ZERO, |acc, next| acc + next.routed_emission)
    }

    #[test]
    fn neutral_scores_preserve_base_alloc_routing() {
        let router = GovernanceEmissionRouter::default();
        let allocations = router
            .route_fixed_budget(
                Amount::new(1_000),
                &[
                    GovernancePoolInput::new(75, 5_000),
                    GovernancePoolInput::new(25, 5_000),
                ],
            )
            .unwrap();

        assert_eq!(allocations[0].governance_multiplier_bps, 10_000);
        assert_eq!(allocations[1].governance_multiplier_bps, 10_000);
        assert_eq!(allocations[0].routed_emission, Amount::new(750));
        assert_eq!(allocations[1].routed_emission, Amount::new(250));
        assert_eq!(sum_emissions(&allocations), Amount::new(1_000));
    }

    #[test]
    fn governance_tilt_redistributes_without_expanding_budget() {
        let router = GovernanceEmissionRouter::default();
        let allocations = router
            .route_fixed_budget(
                Amount::new(1_000),
                &[
                    GovernancePoolInput::new(100, 8_000),
                    GovernancePoolInput::new(100, 2_000),
                ],
            )
            .unwrap();

        assert_eq!(allocations[0].governance_multiplier_bps, 11_200);
        assert_eq!(allocations[1].governance_multiplier_bps, 8_800);
        assert_eq!(allocations[0].routed_emission, Amount::new(560));
        assert_eq!(allocations[1].routed_emission, Amount::new(440));
        assert_eq!(sum_emissions(&allocations), Amount::new(1_000));
    }

    #[test]
    fn remainder_distribution_keeps_total_budget_exact() {
        let router = GovernanceEmissionRouter::default();
        let allocations = router
            .route_fixed_budget(
                Amount::new(1_001),
                &[
                    GovernancePoolInput::new(5, 8_000),
                    GovernancePoolInput::new(3, 5_000),
                    GovernancePoolInput::new(2, 2_000),
                ],
            )
            .unwrap();

        assert_eq!(sum_emissions(&allocations), Amount::new(1_001));
        assert_eq!(
            allocations
                .iter()
                .map(|allocation| allocation.routed_emission.0)
                .collect::<Vec<_>>(),
            vec![541, 290, 170]
        );
    }

    #[test]
    fn normalized_routing_avoids_protocol_wide_emission_expansion() {
        let router = GovernanceEmissionRouter::default();
        let budget = Amount::new(1_000);
        let allocations = router
            .route_fixed_budget(
                budget,
                &[
                    GovernancePoolInput::new(100, 9_000),
                    GovernancePoolInput::new(100, 9_000),
                ],
            )
            .unwrap();

        let naive_expansion = 500u128
            .checked_mul(u128::from(router.governance_multiplier_bps(9_000)))
            .unwrap()
            / u128::from(BPS_SCALE)
            + 500u128
                .checked_mul(u128::from(router.governance_multiplier_bps(9_000)))
                .unwrap()
                / u128::from(BPS_SCALE);

        assert_eq!(naive_expansion, 1_160);
        assert_eq!(sum_emissions(&allocations), budget);
        assert_eq!(allocations[0].routed_emission, Amount::new(500));
        assert_eq!(allocations[1].routed_emission, Amount::new(500));
    }

    #[test]
    fn config_validation_rejects_invalid_neutral_score() {
        let config = GovernanceBoostConfig {
            neutral_governance_bps: 0,
            ..Default::default()
        };
        assert!(GovernanceEmissionRouter::with_config(config).is_err());
    }
}
