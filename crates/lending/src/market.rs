use wrfcoin_defi_common::error::{DefiError, DefiResult};
use wrfcoin_defi_common::traits::LendingProtocol;
use wrfcoin_defi_common::types::{Address, Amount, AssetId, LiquidationEvent, Timestamp};

use crate::collateral::{CollateralConfig, CollateralManager};
use crate::flash_loan::{self, DEFAULT_FLASH_LOAN_FEE_BPS};
use crate::interest::LinearInterestRate;
use crate::liquidation::LiquidationEngine;
use crate::oracle::SimplePriceOracle;
use crate::pool::LendingPool;

/// Unified lending market composing all lending components.
pub struct LendingMarket {
    pub pool: LendingPool,
    pub collateral: CollateralManager,
    pub liquidation: LiquidationEngine,
    pub interest_model: LinearInterestRate,
    pub oracle: SimplePriceOracle,
    pub flash_loan_fee_bps: u64,
    pub max_price_age_secs: Option<u64>,
}

impl LendingMarket {
    pub fn new() -> Self {
        Self {
            pool: LendingPool::new(),
            collateral: CollateralManager::new(),
            liquidation: LiquidationEngine::default(),
            interest_model: LinearInterestRate::default(),
            oracle: SimplePriceOracle::new(),
            flash_loan_fee_bps: DEFAULT_FLASH_LOAN_FEE_BPS,
            max_price_age_secs: None,
        }
    }

    pub fn init_reserve(&mut self, asset: AssetId, timestamp: Timestamp) {
        self.pool.init_reserve(asset, timestamp);
    }

    pub fn set_max_price_age_secs(&mut self, max_age_secs: u64) {
        self.max_price_age_secs = Some(max_age_secs);
    }

    pub fn accrue_interest(&mut self, asset: &AssetId, timestamp: Timestamp) -> DefiResult<()> {
        self.pool
            .accrue_interest(asset, &self.interest_model, timestamp)
    }

    pub fn flash_loan<F>(
        &mut self,
        asset: &AssetId,
        amount: Amount,
        timestamp: Timestamp,
        callback: F,
    ) -> DefiResult<Amount>
    where
        F: FnOnce(Amount) -> DefiResult<Amount>,
    {
        flash_loan::execute_flash_loan(
            &mut self.pool,
            asset,
            amount,
            self.flash_loan_fee_bps,
            timestamp,
            callback,
        )
    }

    pub fn health_factor(&self, borrower: &Address) -> DefiResult<u64> {
        self.collateral.get_health_factor(borrower, &self.oracle)
    }

    pub fn add_collateral(
        &mut self,
        borrower: Address,
        asset: AssetId,
        amount: Amount,
        config: CollateralConfig,
    ) -> DefiResult<()> {
        self.collateral.add_collateral(borrower, asset, amount, config)
    }

    pub fn available_liquidity(&self, asset: &AssetId) -> DefiResult<Amount> {
        self.pool.available_liquidity(asset)
    }

    fn ensure_fresh_prices(&self, assets: &[AssetId], timestamp: Timestamp) -> DefiResult<()> {
        if let Some(max_age_secs) = self.max_price_age_secs {
            self.oracle.ensure_fresh(assets, timestamp, max_age_secs)?;
        }
        Ok(())
    }
}

impl Default for LendingMarket {
    fn default() -> Self {
        Self::new()
    }
}

impl LendingProtocol for LendingMarket {
    fn deposit(
        &mut self,
        _depositor: Address,
        asset: AssetId,
        amount: Amount,
        timestamp: Timestamp,
    ) -> DefiResult<()> {
        self.accrue_interest(&asset, timestamp)?;
        self.pool.deposit(&asset, amount, timestamp)
    }

    fn borrow(
        &mut self,
        borrower: Address,
        asset: AssetId,
        amount: Amount,
        timestamp: Timestamp,
    ) -> DefiResult<()> {
        self.ensure_fresh_prices(std::slice::from_ref(&asset), timestamp)?;
        self.accrue_interest(&asset, timestamp)?;

        self.pool.borrow(&asset, amount, timestamp)?;

        let debt_key = (borrower, asset.clone());
        let current_debt = self
            .collateral
            .debts
            .get(&debt_key)
            .copied()
            .unwrap_or(Amount::ZERO);
        let new_debt = current_debt.checked_add(amount).ok_or(DefiError::Overflow {
            operation: "market borrow debt tracking".into(),
        })?;
        self.collateral.debts.insert(debt_key.clone(), new_debt);

        let collateral_assets = self
            .collateral
            .positions
            .get(&borrower)
            .map(|positions| positions.keys().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        self.ensure_fresh_prices(&collateral_assets, timestamp)?;

        let hf = self.collateral.get_health_factor(&borrower, &self.oracle)?;
        if hf < self.liquidation.liquidation_threshold_bps {
            self.pool.repay(&asset, amount, timestamp)?;
            self.collateral.debts.insert(debt_key, current_debt);
            return Err(DefiError::InsufficientCollateral {
                health_factor_bps: hf,
            });
        }

        Ok(())
    }

    fn repay(
        &mut self,
        borrower: Address,
        asset: AssetId,
        amount: Amount,
        timestamp: Timestamp,
    ) -> DefiResult<()> {
        self.accrue_interest(&asset, timestamp)?;

        let repaid = self.pool.repay(&asset, amount, timestamp)?;

        let debt_key = (borrower, asset);
        if let Some(debt) = self.collateral.debts.get_mut(&debt_key) {
            *debt = debt.checked_sub(repaid).ok_or(DefiError::Overflow {
                operation: "market repay debt tracking".into(),
            })?;
            if debt.is_zero() {
                self.collateral.debts.remove(&debt_key);
            }
        }

        Ok(())
    }

    fn liquidate(
        &mut self,
        liquidator: Address,
        borrower: Address,
        debt_asset: AssetId,
        collateral_asset: AssetId,
        amount: Amount,
        timestamp: Timestamp,
    ) -> DefiResult<LiquidationEvent> {
        self.ensure_fresh_prices(&[debt_asset.clone(), collateral_asset.clone()], timestamp)?;
        self.accrue_interest(&debt_asset, timestamp)?;

        let outcome = self.liquidation.liquidate(
            liquidator,
            &borrower,
            &debt_asset,
            &collateral_asset,
            amount,
            &mut self.collateral,
            &self.oracle,
        )?;

        self.pool.repay(&debt_asset, outcome.debt_repaid, timestamp)?;

        Ok(LiquidationEvent::new(
            LiquidationEvent::borrower_position_id(&borrower),
            outcome.collateral_seized,
            outcome.debt_repaid,
            liquidator,
            timestamp,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wrfcoin_defi_common::types::Price;

    fn ts(secs: u64) -> Timestamp {
        Timestamp::new(secs)
    }

    fn asset(name: &str) -> AssetId {
        AssetId::new(name)
    }

    fn addr(n: u8) -> Address {
        let mut bytes = [0u8; 32];
        bytes[31] = n;
        Address::from_bytes(bytes)
    }

    fn config(max_ltv_bps: u64, liquidation_threshold_bps: u64) -> CollateralConfig {
        CollateralConfig::new(max_ltv_bps, liquidation_threshold_bps).expect("test collateral config should be valid")
    }

    fn setup_market() -> LendingMarket {
        let mut market = LendingMarket::new();
        market.init_reserve(asset("USDC"), ts(0));
        market.init_reserve(asset("ETH"), ts(0));
        market
            .oracle
            .set_price_at(asset("USDC"), Price::from_integer(1), ts(0));
        market
            .oracle
            .set_price_at(asset("ETH"), Price::from_integer(2000), ts(0));
        market
    }

    #[test]
    fn test_deposit_and_borrow_flow() {
        let mut market = setup_market();
        market
            .deposit(addr(1), asset("USDC"), Amount::new(100_000), ts(1))
            .expect("market operation should succeed");
        market
            .add_collateral(addr(2), asset("ETH"), Amount::new(1), config(7_500, 8_000))
            .expect("market operation should succeed");
        market
            .borrow(addr(2), asset("USDC"), Amount::new(1_000), ts(2))
            .expect("market operation should succeed");

        assert_eq!(
            market.collateral.debts[&(addr(2), asset("USDC"))],
            Amount::new(1_000)
        );
        assert_eq!(
            market.pool.available_liquidity(&asset("USDC")).expect("available_liquidity should succeed"),
            Amount::new(99_000)
        );
    }

    #[test]
    fn test_borrow_exceeds_health_factor_fails() {
        let mut market = setup_market();

        market
            .deposit(addr(1), asset("USDC"), Amount::new(100_000), ts(1))
            .expect("market operation should succeed");
        market
            .add_collateral(addr(2), asset("ETH"), Amount::new(1), config(7_500, 8_000))
            .expect("market operation should succeed");

        let result = market.borrow(addr(2), asset("USDC"), Amount::new(2_000), ts(2));
        assert!(result.is_err());
        assert_eq!(
            market.pool.reserves[&asset("USDC")].total_borrows,
            Amount::ZERO
        );
    }

    #[test]
    fn test_repay_flow() {
        let mut market = setup_market();
        market
            .deposit(addr(1), asset("USDC"), Amount::new(100_000), ts(1))
            .expect("market operation should succeed");
        market
            .add_collateral(addr(2), asset("ETH"), Amount::new(1), config(7_500, 8_000))
            .expect("market operation should succeed");
        market
            .borrow(addr(2), asset("USDC"), Amount::new(1_000), ts(2))
            .expect("market operation should succeed");
        market
            .repay(addr(2), asset("USDC"), Amount::new(500), ts(3))
            .expect("market operation should succeed");

        assert_eq!(
            market.collateral.debts[&(addr(2), asset("USDC"))],
            Amount::new(500)
        );
        assert_eq!(
            market.pool.reserves[&asset("USDC")].total_borrows,
            Amount::new(500)
        );
    }

    #[test]
    fn test_repay_full_removes_debt() {
        let mut market = setup_market();
        market
            .deposit(addr(1), asset("USDC"), Amount::new(100_000), ts(1))
            .expect("market operation should succeed");
        market
            .add_collateral(addr(2), asset("ETH"), Amount::new(1), config(7_500, 8_000))
            .expect("market operation should succeed");
        market
            .borrow(addr(2), asset("USDC"), Amount::new(1_000), ts(2))
            .expect("market operation should succeed");
        market
            .repay(addr(2), asset("USDC"), Amount::new(1_000), ts(3))
            .expect("market operation should succeed");

        assert!(!market.collateral.debts.contains_key(&(addr(2), asset("USDC"))));
        assert_eq!(
            market.pool.reserves[&asset("USDC")].total_borrows,
            Amount::ZERO
        );
    }

    #[test]
    fn test_liquidation_through_market() {
        let mut market = setup_market();
        market
            .deposit(addr(1), asset("USDC"), Amount::new(100_000), ts(1))
            .expect("market operation should succeed");
        market
            .add_collateral(addr(2), asset("ETH"), Amount::new(500), config(7_500, 8_000))
            .expect("market operation should succeed");
        market
            .borrow(addr(2), asset("USDC"), Amount::new(1_000), ts(2))
            .expect("market operation should succeed");

        market
            .oracle
            .set_price_at(asset("ETH"), Price::from_integer(1), ts(3));

        let event = market
            .liquidate(
                addr(3),
                addr(2),
                asset("USDC"),
                asset("ETH"),
                Amount::new(200),
                ts(3),
            )
            .expect("market operation should succeed");

        assert_eq!(event.collateral_seized, Amount::new(210));
        assert_eq!(event.debt_repaid, Amount::new(200));
        assert_eq!(event.timestamp, ts(3));
        assert_eq!(
            market.collateral.debts[&(addr(2), asset("USDC"))],
            Amount::new(800)
        );
    }

    #[test]
    fn test_interest_accrual_over_time() {
        let mut market = setup_market();
        market
            .deposit(addr(1), asset("USDC"), Amount::new(100_000), ts(0))
            .expect("market operation should succeed");
        market
            .add_collateral(addr(2), asset("ETH"), Amount::new(100), config(7_500, 8_000))
            .expect("market operation should succeed");
        market
            .borrow(addr(2), asset("USDC"), Amount::new(50_000), ts(0))
            .expect("market operation should succeed");

        let borrows_before = market.pool.reserves[&asset("USDC")].total_borrows;
        assert_eq!(borrows_before, Amount::new(50_000));

        let one_year = 365 * 24 * 3600;
        market.accrue_interest(&asset("USDC"), ts(one_year)).expect("accrue_interest should succeed");

        let borrows_after = market.pool.reserves[&asset("USDC")].total_borrows;
        assert_eq!(borrows_after, Amount::new(52_250));
    }

    #[test]
    fn test_flash_loan_through_market() {
        let mut market = setup_market();
        market
            .deposit(addr(1), asset("USDC"), Amount::new(100_000), ts(1))
            .expect("market operation should succeed");

        let fee = market
            .flash_loan(&asset("USDC"), Amount::new(50_000), ts(2), |_borrowed| {
                Ok(Amount::new(50_045))
            })
            .expect("market operation should succeed");

        assert_eq!(fee, Amount::new(45));
        assert_eq!(
            market.pool.reserves[&asset("USDC")].total_deposits,
            Amount::new(100_045)
        );
    }

    #[test]
    fn test_health_factor_no_debt() {
        let market = setup_market();
        assert_eq!(market.health_factor(&addr(1)).expect("health_factor should succeed"), u64::MAX);
    }

    #[test]
    fn test_stale_oracle_rejects_borrow() {
        let mut market = setup_market();
        market.set_max_price_age_secs(5);
        market
            .deposit(addr(1), asset("USDC"), Amount::new(100_000), ts(1))
            .expect("market operation should succeed");
        market
            .add_collateral(addr(2), asset("ETH"), Amount::new(1), config(7_500, 8_000))
            .expect("market operation should succeed");

        let result = market.borrow(addr(2), asset("USDC"), Amount::new(100), ts(10));
        assert!(matches!(result, Err(DefiError::OracleError { .. })));
    }

    #[test]
    fn test_fresh_oracle_allows_borrow() {
        let mut market = setup_market();
        market.set_max_price_age_secs(10);
        market
            .oracle
            .set_price_at(asset("USDC"), Price::from_integer(1), ts(9));
        market
            .oracle
            .set_price_at(asset("ETH"), Price::from_integer(2_000), ts(9));
        market
            .deposit(addr(1), asset("USDC"), Amount::new(100_000), ts(1))
            .expect("market operation should succeed");
        market
            .add_collateral(addr(2), asset("ETH"), Amount::new(1), config(7_500, 8_000))
            .expect("market operation should succeed");

        assert!(market
            .borrow(addr(2), asset("USDC"), Amount::new(100), ts(10))
            .is_ok());
    }
}
