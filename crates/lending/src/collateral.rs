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
