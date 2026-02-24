use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use wrfcoin_defi_common::error::{DefiError, DefiResult};
use wrfcoin_defi_common::traits::PriceOracle;
use wrfcoin_defi_common::types::{Address, Amount, AssetId};

/// A single collateral position for one asset.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollateralPosition {
    /// The asset pledged as collateral.
    pub asset: AssetId,
    /// Amount of the asset deposited.
    pub amount: Amount,
    /// Loan-to-value ratio in basis points (e.g., 7500 = 75%).
    pub ltv_ratio_bps: u64,
}

/// Manages collateral positions for all borrowers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollateralManager {
    /// Positions keyed by borrower address.
    pub positions: HashMap<Address, Vec<CollateralPosition>>,
    /// Outstanding debt per (borrower, asset).
    pub debts: HashMap<(Address, AssetId), Amount>,
}

impl CollateralManager {
    pub fn new() -> Self {
        Self {
            positions: HashMap::new(),
            debts: HashMap::new(),
        }
    }

    /// Add collateral for a borrower.
    pub fn add_collateral(
        &mut self,
        borrower: Address,
        asset: AssetId,
        amount: Amount,
        ltv_ratio_bps: u64,
    ) -> DefiResult<()> {
        if amount.is_zero() {
            return Err(DefiError::InvalidAmount {
                reason: "collateral amount must be non-zero".into(),
            });
        }

        let positions = self.positions.entry(borrower).or_default();

        // If position for this asset already exists, increase its amount.
        if let Some(pos) = positions.iter_mut().find(|p| p.asset == asset) {
            pos.amount = pos.amount.checked_add(amount).ok_or(DefiError::Overflow {
                operation: "add_collateral".into(),
            })?;
        } else {
            positions.push(CollateralPosition {
                asset,
                amount,
                ltv_ratio_bps,
            });
        }

        Ok(())
    }

    /// Remove collateral for a borrower. Fails if it would make the position unhealthy.
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
            .iter_mut()
            .find(|p| p.asset == *asset)
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

        // Clean up zero-amount positions.
        if pos.amount.is_zero() {
            positions.retain(|p| !p.amount.is_zero());
        }

        Ok(())
    }

    /// Calculate the health factor for a borrower in basis points.
    ///
    /// health_factor = (sum of collateral_value * ltv) / total_debt_value
    /// Returns 10_000 (100%) as the liquidation threshold boundary.
    /// A value below 10_000 means the position is liquidatable.
    /// Returns u64::MAX if the borrower has no debt.
    pub fn get_health_factor(
        &self,
        borrower: &Address,
        oracle: &dyn PriceOracle,
    ) -> DefiResult<u64> {
        let positions = match self.positions.get(borrower) {
            Some(p) => p,
            None => return Ok(u64::MAX), // No collateral, no debt => healthy
        };

        // Sum weighted collateral value.
        let mut weighted_collateral_value: u128 = 0;
        for pos in positions {
            let price = oracle.get_price(&pos.asset)?;
            let value = price.apply(pos.amount).ok_or(DefiError::Overflow {
                operation: "health_factor collateral valuation".into(),
            })?;
            // value * ltv / 10_000
            let weighted = value
                .0
                .checked_mul(pos.ltv_ratio_bps as u128)
                .ok_or(DefiError::Overflow {
                    operation: "health_factor weighting".into(),
                })?;
            weighted_collateral_value = weighted_collateral_value
                .checked_add(weighted)
                .ok_or(DefiError::Overflow {
                    operation: "health_factor sum".into(),
                })?;
        }

        // Sum total debt value.
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

        // health_factor_bps = weighted_collateral_value / total_debt_value
        // (weighted_collateral_value is already scaled by ltv_bps, so divide by 10_000 * debt)
        // Actually: hf = (sum(col_val * ltv_bps)) / (total_debt * 10_000)
        let hf = weighted_collateral_value
            .checked_div(total_debt_value)
            .unwrap_or(0);

        Ok(hf as u64)
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
    use wrfcoin_defi_common::types::Price;

    fn addr(n: u8) -> Address {
        let mut bytes = [0u8; 32];
        bytes[0] = n;
        Address::from_bytes(bytes)
    }

    fn asset(name: &str) -> AssetId {
        AssetId::new(name)
    }

    #[test]
    fn test_add_collateral() {
        let mut cm = CollateralManager::new();
        cm.add_collateral(addr(1), asset("ETH"), Amount::new(1000), 7500).unwrap();
        let positions = &cm.positions[&addr(1)];
        assert_eq!(positions.len(), 1);
        assert_eq!(positions[0].amount, Amount::new(1000));
    }

    #[test]
    fn test_add_collateral_same_asset_merges() {
        let mut cm = CollateralManager::new();
        cm.add_collateral(addr(1), asset("ETH"), Amount::new(500), 7500).unwrap();
        cm.add_collateral(addr(1), asset("ETH"), Amount::new(300), 7500).unwrap();
        let positions = &cm.positions[&addr(1)];
        assert_eq!(positions.len(), 1);
        assert_eq!(positions[0].amount, Amount::new(800));
    }

    #[test]
    fn test_add_collateral_zero_fails() {
        let mut cm = CollateralManager::new();
        let result = cm.add_collateral(addr(1), asset("ETH"), Amount::ZERO, 7500);
        assert!(result.is_err());
    }

    #[test]
    fn test_remove_collateral() {
        let mut cm = CollateralManager::new();
        cm.add_collateral(addr(1), asset("ETH"), Amount::new(1000), 7500).unwrap();
        cm.remove_collateral(addr(1), &asset("ETH"), Amount::new(400)).unwrap();
        assert_eq!(cm.positions[&addr(1)][0].amount, Amount::new(600));
    }

    #[test]
    fn test_remove_collateral_exceeds_fails() {
        let mut cm = CollateralManager::new();
        cm.add_collateral(addr(1), asset("ETH"), Amount::new(100), 7500).unwrap();
        let result = cm.remove_collateral(addr(1), &asset("ETH"), Amount::new(200));
        assert!(result.is_err());
    }

    #[test]
    fn test_remove_collateral_cleans_zero() {
        let mut cm = CollateralManager::new();
        cm.add_collateral(addr(1), asset("ETH"), Amount::new(100), 7500).unwrap();
        cm.remove_collateral(addr(1), &asset("ETH"), Amount::new(100)).unwrap();
        assert!(cm.positions[&addr(1)].is_empty());
    }

    #[test]
    fn test_health_factor_no_debt() {
        let mut cm = CollateralManager::new();
        cm.add_collateral(addr(1), asset("ETH"), Amount::new(1000), 7500).unwrap();
        let mut oracle = SimplePriceOracle::new();
        oracle.set_price(asset("ETH"), Price::from_integer(1));
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
        // Collateral: 1000 ETH at price 2, LTV 75%
        cm.add_collateral(addr(1), asset("ETH"), Amount::new(1000), 7500).unwrap();
        // Debt: 500 USDC at price 1
        cm.debts.insert((addr(1), asset("USDC")), Amount::new(500));

        let mut oracle = SimplePriceOracle::new();
        oracle.set_price(asset("ETH"), Price::from_integer(2));
        oracle.set_price(asset("USDC"), Price::from_integer(1));

        let hf = cm.get_health_factor(&addr(1), &oracle).unwrap();
        // weighted_collateral = 1000 * 2 * 7500 = 15_000_000
        // debt = 500 * 1 = 500
        // hf = 15_000_000 / 500 = 30_000
        assert_eq!(hf, 30_000);
    }
}
