use serde::{Deserialize, Serialize};
use wrfcoin_defi_common::error::{DefiError, DefiResult};
use wrfcoin_defi_common::traits::PriceOracle;
use wrfcoin_defi_common::types::{Address, Amount, AssetId};

use crate::collateral::CollateralManager;

/// Result of a liquidation before adapter/event projection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiquidationOutcome {
    pub collateral_seized: Amount,
    pub debt_repaid: Amount,
}

/// Engine responsible for checking and executing liquidations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiquidationEngine {
    pub liquidation_threshold_bps: u64,
    pub liquidation_bonus_bps: u64,
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

    pub fn check_liquidatable(
        &self,
        borrower: &Address,
        collateral_manager: &CollateralManager,
        oracle: &dyn PriceOracle,
    ) -> DefiResult<bool> {
        let health_factor = collateral_manager.get_health_factor(borrower, oracle)?;
        Ok(health_factor < self.liquidation_threshold_bps)
    }

    pub fn calculate_liquidation_amount(&self, debt_amount: Amount) -> DefiResult<Amount> {
        debt_amount
            .mul_fraction(self.close_factor_bps as u128, 10_000)
            .ok_or(DefiError::Overflow {
                operation: "calculate_liquidation_amount".into(),
            })
    }

    pub fn calculate_collateral_reward(
        &self,
        debt_repaid: Amount,
        debt_asset: &AssetId,
        collateral_asset: &AssetId,
        oracle: &dyn PriceOracle,
    ) -> DefiResult<Amount> {
        let debt_price = oracle.get_price(debt_asset)?;
        let collateral_price = oracle.get_price(collateral_asset)?;

        let debt_value = debt_price.apply(debt_repaid).ok_or(DefiError::Overflow {
            operation: "liquidation debt valuation".into(),
        })?;

        let bonus_multiplier = 10_000u128 + self.liquidation_bonus_bps as u128;
        let rewarded_value = debt_value
            .mul_fraction(bonus_multiplier, 10_000)
            .ok_or(DefiError::Overflow {
                operation: "liquidation bonus".into(),
            })?;

        let inverse_price = collateral_price.inverse().ok_or(DefiError::OracleError {
            message: "collateral price is zero".into(),
        })?;

        inverse_price.apply(rewarded_value).ok_or(DefiError::Overflow {
            operation: "liquidation collateral conversion".into(),
        })
    }

    pub fn liquidate(
        &self,
        _liquidator: Address,
        borrower: &Address,
        debt_asset: &AssetId,
        collateral_asset: &AssetId,
        repay_amount: Amount,
        collateral_manager: &mut CollateralManager,
        oracle: &dyn PriceOracle,
    ) -> DefiResult<LiquidationOutcome> {
        if !self.check_liquidatable(borrower, collateral_manager, oracle)? {
            return Err(DefiError::InsufficientCollateral {
                health_factor_bps: collateral_manager.get_health_factor(borrower, oracle)?,
            });
        }

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

        let collateral_reward =
            self.calculate_collateral_reward(repay_amount, debt_asset, collateral_asset, oracle)?;

        if let Some(debt) = collateral_manager.debts.get_mut(&debt_key) {
            *debt = debt.checked_sub(repay_amount).ok_or(DefiError::Overflow {
                operation: "liquidation debt reduction".into(),
            })?;
        }

        collateral_manager.remove_collateral(*borrower, collateral_asset, collateral_reward)?;

        Ok(LiquidationOutcome {
            collateral_seized: collateral_reward,
            debt_repaid: repay_amount,
        })
    }
}

impl Default for LiquidationEngine {
    fn default() -> Self {
        Self::new(10_000, 500, 5_000)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collateral::{CollateralConfig, CollateralManager};
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

    fn setup_undercollateralized() -> (CollateralManager, SimplePriceOracle) {
        let mut cm = CollateralManager::new();
        cm.add_collateral(
            addr(1),
            asset("ETH"),
            Amount::new(500),
            CollateralConfig::new(7_500, 8_000).expect("valid collateral config"),
        )
        .expect("add_collateral should succeed");
        cm.debts.insert((addr(1), asset("USDC")), Amount::new(600));

        let mut oracle = SimplePriceOracle::new();
        oracle.set_price_at(asset("ETH"), Price::from_integer(1), Timestamp::new(1));
        oracle.set_price_at(asset("USDC"), Price::from_integer(1), Timestamp::new(1));
        (cm, oracle)
    }

    #[test]
    fn test_check_liquidatable() {
        let (cm, oracle) = setup_undercollateralized();
        let engine = LiquidationEngine::default();
        assert!(engine.check_liquidatable(&addr(1), &cm, &oracle).expect("check_liquidatable should succeed"));
    }

    #[test]
    fn test_check_not_liquidatable() {
        let mut cm = CollateralManager::new();
        cm.add_collateral(
            addr(1),
            asset("ETH"),
            Amount::new(10_000),
            CollateralConfig::new(7_500, 8_000).expect("valid collateral config"),
        )
        .expect("add_collateral should succeed");
        cm.debts.insert((addr(1), asset("USDC")), Amount::new(1));
        let mut oracle = SimplePriceOracle::new();
        oracle.set_price_at(asset("ETH"), Price::from_integer(1), Timestamp::new(1));
        oracle.set_price_at(asset("USDC"), Price::from_integer(1), Timestamp::new(1));

        let engine = LiquidationEngine::default();
        assert!(!engine.check_liquidatable(&addr(1), &cm, &oracle).expect("check_liquidatable should succeed"));
    }

    #[test]
    fn test_calculate_liquidation_amount() {
        let engine = LiquidationEngine::default();
        let max = engine.calculate_liquidation_amount(Amount::new(1000)).expect("liquidation amount calculation should succeed");
        assert_eq!(max, Amount::new(500));
    }

    #[test]
    fn test_calculate_collateral_reward() {
        let engine = LiquidationEngine::new(10_000, 500, 5_000);
        let mut oracle = SimplePriceOracle::new();
        oracle.set_price_at(asset("USDC"), Price::from_integer(1), Timestamp::new(1));
        oracle.set_price_at(asset("ETH"), Price::from_integer(2), Timestamp::new(1));

        let reward = engine
            .calculate_collateral_reward(Amount::new(100), &asset("USDC"), &asset("ETH"), &oracle)
            .expect("collateral reward calculation should succeed");
        assert_eq!(reward, Amount::new(52));
    }

    #[test]
    fn test_liquidate_full_flow() {
        let (mut cm, oracle) = setup_undercollateralized();
        let engine = LiquidationEngine::default();

        let outcome = engine
            .liquidate(
                addr(2),
                &addr(1),
                &asset("USDC"),
                &asset("ETH"),
                Amount::new(200),
                &mut cm,
                &oracle,
            )
            .expect("liquidation should succeed for undercollateralized position");

        assert_eq!(cm.debts[&(addr(1), asset("USDC"))], Amount::new(400));
        assert_eq!(outcome.collateral_seized, Amount::new(210));
        assert_eq!(outcome.debt_repaid, Amount::new(200));
    }

    #[test]
    fn test_liquidate_healthy_position_fails() {
        let mut cm = CollateralManager::new();
        cm.add_collateral(
            addr(1),
            asset("ETH"),
            Amount::new(10_000),
            CollateralConfig::new(7_500, 8_000).expect("valid collateral config"),
        )
        .expect("add_collateral should succeed");
        cm.debts.insert((addr(1), asset("USDC")), Amount::new(1));
        let mut oracle = SimplePriceOracle::new();
        oracle.set_price_at(asset("ETH"), Price::from_integer(1), Timestamp::new(1));
        oracle.set_price_at(asset("USDC"), Price::from_integer(1), Timestamp::new(1));

        let engine = LiquidationEngine::default();
        let result = engine.liquidate(
            addr(2),
            &addr(1),
            &asset("USDC"),
            &asset("ETH"),
            Amount::new(1),
            &mut cm,
            &oracle,
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_liquidate_exceeds_close_factor_fails() {
        let (mut cm, oracle) = setup_undercollateralized();
        let engine = LiquidationEngine::default();
        let result = engine.liquidate(
            addr(2),
            &addr(1),
            &asset("USDC"),
            &asset("ETH"),
            Amount::new(400),
            &mut cm,
            &oracle,
        );
        assert!(result.is_err());
    }
}
