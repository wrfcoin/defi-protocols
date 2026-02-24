use serde::{Deserialize, Serialize};
use wrfcoin_defi_common::error::{DefiError, DefiResult};
use wrfcoin_defi_common::traits::PriceOracle;
use wrfcoin_defi_common::types::{Address, Amount, AssetId};

use crate::collateral::CollateralManager;

/// Engine responsible for checking and executing liquidations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiquidationEngine {
    /// Health factor threshold (in bps) below which a position can be liquidated.
    /// Typically 10_000 (i.e., 100% — when collateral value * LTV < debt value).
    pub liquidation_threshold_bps: u64,
    /// Bonus given to liquidators in basis points (e.g., 500 = 5%).
    pub liquidation_bonus_bps: u64,
    /// Maximum fraction of debt that can be liquidated at once, in bps.
    pub close_factor_bps: u64,
}

impl LiquidationEngine {
    pub fn new(
        liquidation_threshold_bps: u64,
        liquidation_bonus_bps: u64,
        close_factor_bps: u64,
    ) -> Self {
        Self {
            liquidation_threshold_bps,
            liquidation_bonus_bps,
            close_factor_bps,
        }
    }

    /// Check whether a borrower's position is eligible for liquidation.
    pub fn check_liquidatable(
        &self,
        borrower: &Address,
        collateral_manager: &CollateralManager,
        oracle: &dyn PriceOracle,
    ) -> DefiResult<bool> {
        let health_factor = collateral_manager.get_health_factor(borrower, oracle)?;
        Ok(health_factor < self.liquidation_threshold_bps)
    }

    /// Calculate the maximum amount of debt that can be repaid in a liquidation.
    pub fn calculate_liquidation_amount(
        &self,
        debt_amount: Amount,
    ) -> DefiResult<Amount> {
        // Liquidator can repay up to close_factor of the debt.
        debt_amount
            .mul_fraction(self.close_factor_bps as u128, 10_000)
            .ok_or(DefiError::Overflow {
                operation: "calculate_liquidation_amount".into(),
            })
    }

    /// Calculate the collateral amount the liquidator receives, including the bonus.
    pub fn calculate_collateral_reward(
        &self,
        debt_repaid: Amount,
        debt_asset: &AssetId,
        collateral_asset: &AssetId,
        oracle: &dyn PriceOracle,
    ) -> DefiResult<Amount> {
        let debt_price = oracle.get_price(debt_asset)?;
        let collateral_price = oracle.get_price(collateral_asset)?;

        // Convert debt amount to its base-currency value.
        let debt_value = debt_price.apply(debt_repaid).ok_or(DefiError::Overflow {
            operation: "liquidation debt valuation".into(),
        })?;

        // Apply liquidation bonus: value * (10_000 + bonus) / 10_000.
        let bonus_multiplier = 10_000u128 + self.liquidation_bonus_bps as u128;
        let rewarded_value = debt_value
            .mul_fraction(bonus_multiplier, 10_000)
            .ok_or(DefiError::Overflow {
                operation: "liquidation bonus".into(),
            })?;

        // Convert rewarded value back to collateral asset units.
        let inverse_price = collateral_price.inverse().ok_or(DefiError::OracleError {
            message: "collateral price is zero".into(),
        })?;

        inverse_price.apply(rewarded_value).ok_or(DefiError::Overflow {
            operation: "liquidation collateral conversion".into(),
        })
    }

    /// Execute a liquidation: verify eligibility, calculate amounts, and modify state.
    pub fn liquidate(
        &self,
        _liquidator: Address,
        borrower: &Address,
        debt_asset: &AssetId,
        collateral_asset: &AssetId,
        repay_amount: Amount,
        collateral_manager: &mut CollateralManager,
        oracle: &dyn PriceOracle,
    ) -> DefiResult<Amount> {
        // Verify the position is liquidatable.
        if !self.check_liquidatable(borrower, collateral_manager, oracle)? {
            return Err(DefiError::InsufficientCollateral {
                health_factor_bps: collateral_manager.get_health_factor(borrower, oracle)?,
            });
        }

        // Check the repay amount does not exceed the close factor.
        let debt_key = (*borrower, debt_asset.clone());
        let total_debt = collateral_manager
            .debts
            .get(&debt_key)
            .copied()
            .unwrap_or(Amount::ZERO);

        let max_repay = self.calculate_liquidation_amount(total_debt)?;
        if repay_amount > max_repay {
            return Err(DefiError::InvalidAmount {
                reason: format!(
                    "repay amount {} exceeds max liquidation amount {}",
                    repay_amount, max_repay
                ),
            });
        }

        // Calculate collateral reward.
        let collateral_reward =
            self.calculate_collateral_reward(repay_amount, debt_asset, collateral_asset, oracle)?;

        // Update debt.
        if let Some(debt) = collateral_manager.debts.get_mut(&debt_key) {
            *debt = debt.checked_sub(repay_amount).ok_or(DefiError::Overflow {
                operation: "liquidation debt reduction".into(),
            })?;
        }

        // Seize collateral.
        collateral_manager.remove_collateral(*borrower, collateral_asset, collateral_reward)?;

        Ok(collateral_reward)
    }
}

impl Default for LiquidationEngine {
    fn default() -> Self {
        Self::new(10_000, 500, 5_000)
    }
}
