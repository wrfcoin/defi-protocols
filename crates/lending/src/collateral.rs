use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use wrfcoin_defi_common::error::{DefiError, DefiResult};
use wrfcoin_defi_common::traits::PriceOracle;
use wrfcoin_defi_common::types::{Address, Amount, AssetId};

pub const MAX_SAFE_LTV_BPS: u64 = 9_000;
pub const MAX_LIQUIDATION_THRESHOLD_BPS: u64 = 10_000;

/// Risk configuration for a collateral asset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CollateralConfig {
    pub max_ltv_bps: u64,
    pub liquidation_threshold_bps: u64,
}

impl CollateralConfig {
    pub fn new(max_ltv_bps: u64, liquidation_threshold_bps: u64) -> DefiResult<Self> {
        if max_ltv_bps == 0 || max_ltv_bps > MAX_SAFE_LTV_BPS {
            return Err(DefiError::InvalidAmount {
                reason: format!("max LTV must be between 1 and {MAX_SAFE_LTV_BPS} bps"),
            });
        }

        if liquidation_threshold_bps == 0
            || liquidation_threshold_bps > MAX_LIQUIDATION_THRESHOLD_BPS
        {
            return Err(DefiError::InvalidAmount {
                reason: format!(
                    "liquidation threshold must be between 1 and {MAX_LIQUIDATION_THRESHOLD_BPS} bps"
                ),
            });
        }

        if liquidation_threshold_bps < max_ltv_bps {
            return Err(DefiError::InvalidAmount {
                reason: "liquidation threshold must be >= max LTV".into(),
            });
        }

        Ok(Self {
            max_ltv_bps,
            liquidation_threshold_bps,
        })
    }
}

/// A single collateral position for one asset.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CollateralPosition {
    pub asset: AssetId,
    pub amount: Amount,
    pub config: CollateralConfig,
}

/// Manages collateral positions for all borrowers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollateralManager {
    pub positions: HashMap<Address, HashMap<AssetId, CollateralPosition>>,
    pub debts: HashMap<(Address, AssetId), Amount>,
}

impl CollateralManager {
    pub fn new() -> Self {
        Self {
            positions: HashMap::new(),
            debts: HashMap::new(),
        }
    }

    pub fn add_collateral(
        &mut self,
        borrower: Address,
        asset: AssetId,
        amount: Amount,
        config: CollateralConfig,
    ) -> DefiResult<()> {
        if amount.is_zero() {
            return Err(DefiError::InvalidAmount {
                reason: "collateral amount must be non-zero".into(),
            });
        }

        let positions = self.positions.entry(borrower).or_default();
        if let Some(pos) = positions.get_mut(&asset) {
            pos.amount = pos.amount.checked_add(amount).ok_or(DefiError::Overflow {
                operation: "add_collateral".into(),
            })?;
            pos.config = config;
        } else {
            positions.insert(
                asset.clone(),
                CollateralPosition {
                    asset,
                    amount,
                    config,
                },
            );
        }

        Ok(())
    }

    pub fn remove_collateral(
        &mut self,
        borrower: Address,
        asset: &AssetId,
        amount: Amount,
    ) -> DefiResult<()> {
        if amount.is_zero() {
            return Err(DefiError::InvalidAmount {
                reason: "removal amount must be non-zero".into(),
            });
        }

        let positions = self
            .positions
            .get_mut(&borrower)
            .ok_or_else(|| DefiError::Unauthorized {
                reason: "no collateral positions found".into(),
            })?;

        let pos = positions
            .get_mut(asset)
            .ok_or_else(|| DefiError::InvalidAsset {
                asset_id: asset.clone(),
            })?;

        if amount > pos.amount {
            return Err(DefiError::InsufficientBalance {
                required: amount,
                available: pos.amount,
            });
        }

        pos.amount = pos.amount.checked_sub(amount).ok_or(DefiError::Overflow {
            operation: "remove_collateral".into(),
        })?;

        if pos.amount.is_zero() {
            positions.remove(asset);
        }
        if positions.is_empty() {
            self.positions.remove(&borrower);
        }

        Ok(())
    }

    pub fn get_health_factor(
        &self,
        borrower: &Address,
        oracle: &dyn PriceOracle,
    ) -> DefiResult<u64> {
        let positions = match self.positions.get(borrower) {
            Some(p) => p,
            None => return Ok(u64::MAX),
        };

        let mut weighted_collateral_value: u128 = 0;
        for pos in positions.values() {
            let price = oracle.get_price(&pos.asset)?;
            let value = price.apply(pos.amount).ok_or(DefiError::Overflow {
                operation: "health_factor collateral valuation".into(),
            })?;
            let weighted = value
                .0
                .checked_mul(pos.config.max_ltv_bps as u128)
                .ok_or(DefiError::Overflow {
                    operation: "health_factor weighting".into(),
                })?;
            weighted_collateral_value = weighted_collateral_value
                .checked_add(weighted)
                .ok_or(DefiError::Overflow {
                    operation: "health_factor sum".into(),
                })?;
        }

        let mut total_debt_value: u128 = 0;
        for ((addr, asset), debt_amount) in &self.debts {
            if addr != borrower {
                continue;
            }
            let price = oracle.get_price(asset)?;
            let value = price.apply(*debt_amount).ok_or(DefiError::Overflow {
                operation: "health_factor debt valuation".into(),
            })?;
            total_debt_value = total_debt_value
                .checked_add(value.0)
                .ok_or(DefiError::Overflow {
                    operation: "health_factor debt sum".into(),
                })?;
        }

        if total_debt_value == 0 {
            return Ok(u64::MAX);
        }

        Ok(weighted_collateral_value.checked_div(total_debt_value).unwrap_or(0) as u64)
    }
}

impl Default for CollateralManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oracle::SimplePriceOracle;
    use wrfcoin_defi_common::types::{Price, Timestamp};

    fn addr(n: u8) -> Address {
        let mut bytes = [0u8; 32];
        bytes[31] = n;
        Address::from_bytes(bytes)
    }

    fn asset(name: &str) -> AssetId {
        AssetId::new(name)
    }

    fn config(max_ltv_bps: u64, liquidation_threshold_bps: u64) -> CollateralConfig {
        CollateralConfig::new(max_ltv_bps, liquidation_threshold_bps).unwrap()
    }

    #[test]
    fn test_config_rejects_out_of_bounds_ltv() {
        let result = CollateralConfig::new(MAX_SAFE_LTV_BPS + 1, 9_500);
        assert!(matches!(result, Err(DefiError::InvalidAmount { .. })));
    }

    #[test]
    fn test_config_rejects_threshold_below_ltv() {
        let result = CollateralConfig::new(7_500, 7_000);
        assert!(matches!(result, Err(DefiError::InvalidAmount { .. })));
    }

    #[test]
    fn test_add_collateral() {
        let mut cm = CollateralManager::new();
        cm.add_collateral(addr(1), asset("ETH"), Amount::new(1000), config(7_500, 8_000))
            .unwrap();
        let positions = &cm.positions[&addr(1)];
        assert_eq!(positions.len(), 1);
        assert_eq!(positions[&asset("ETH")].amount, Amount::new(1000));
    }

    #[test]
    fn test_add_collateral_same_asset_merges() {
        let mut cm = CollateralManager::new();
        cm.add_collateral(addr(1), asset("ETH"), Amount::new(500), config(7_500, 8_000))
            .unwrap();
        cm.add_collateral(addr(1), asset("ETH"), Amount::new(300), config(7_500, 8_000))
            .unwrap();
        let positions = &cm.positions[&addr(1)];
        assert_eq!(positions.len(), 1);
        assert_eq!(positions[&asset("ETH")].amount, Amount::new(800));
    }

    #[test]
    fn test_add_collateral_zero_fails() {
        let mut cm = CollateralManager::new();
        let result = cm.add_collateral(
            addr(1),
            asset("ETH"),
            Amount::ZERO,
            config(7_500, 8_000),
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_remove_collateral() {
        let mut cm = CollateralManager::new();
        cm.add_collateral(addr(1), asset("ETH"), Amount::new(1000), config(7_500, 8_000))
            .unwrap();
        cm.remove_collateral(addr(1), &asset("ETH"), Amount::new(400))
            .unwrap();
        assert_eq!(cm.positions[&addr(1)][&asset("ETH")].amount, Amount::new(600));
    }

    #[test]
    fn test_remove_collateral_exceeds_fails() {
        let mut cm = CollateralManager::new();
        cm.add_collateral(addr(1), asset("ETH"), Amount::new(100), config(7_500, 8_000))
            .unwrap();
        let result = cm.remove_collateral(addr(1), &asset("ETH"), Amount::new(200));
        assert!(result.is_err());
    }

    #[test]
    fn test_remove_collateral_cleans_zero() {
        let mut cm = CollateralManager::new();
        cm.add_collateral(addr(1), asset("ETH"), Amount::new(100), config(7_500, 8_000))
            .unwrap();
        cm.remove_collateral(addr(1), &asset("ETH"), Amount::new(100))
            .unwrap();
        assert!(!cm.positions.contains_key(&addr(1)));
    }

    #[test]
    fn test_health_factor_no_debt() {
        let mut cm = CollateralManager::new();
        cm.add_collateral(addr(1), asset("ETH"), Amount::new(1000), config(7_500, 8_000))
            .unwrap();
        let mut oracle = SimplePriceOracle::new();
        oracle.set_price_at(asset("ETH"), Price::from_integer(1), Timestamp::new(1));
        assert_eq!(cm.get_health_factor(&addr(1), &oracle).unwrap(), u64::MAX);
    }

    #[test]
    fn test_health_factor_no_positions() {
        let cm = CollateralManager::new();
        let oracle = SimplePriceOracle::new();
        assert_eq!(cm.get_health_factor(&addr(1), &oracle).unwrap(), u64::MAX);
    }

    #[test]
    fn test_health_factor_with_debt() {
        let mut cm = CollateralManager::new();
        cm.add_collateral(addr(1), asset("ETH"), Amount::new(100), config(7_500, 8_000))
            .unwrap();
        cm.debts.insert((addr(1), asset("USDC")), Amount::new(50));

        let mut oracle = SimplePriceOracle::new();
        oracle.set_price_at(asset("ETH"), Price::from_integer(2), Timestamp::new(1));
        oracle.set_price_at(asset("USDC"), Price::from_integer(1), Timestamp::new(1));

        assert_eq!(cm.get_health_factor(&addr(1), &oracle).unwrap(), 30_000);
    }
}
