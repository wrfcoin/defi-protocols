use wrfcoin_defi_common::error::{DefiError, DefiResult};
use wrfcoin_defi_common::traits::LendingProtocol;
use wrfcoin_defi_common::types::{Address, Amount, AssetId, Timestamp};

use crate::collateral::CollateralManager;
use crate::flash_loan::{self, DEFAULT_FLASH_LOAN_FEE_BPS};
use crate::interest::LinearInterestRate;
use crate::liquidation::LiquidationEngine;
use crate::oracle::SimplePriceOracle;
use crate::pool::LendingPool;

/// Unified lending market composing all lending components.
///
/// This is the high-level API that users interact with. It coordinates the
/// lending pool, collateral manager, liquidation engine, interest rate model,
/// and price oracle.
pub struct LendingMarket {
    pub pool: LendingPool,
    pub collateral: CollateralManager,
    pub liquidation: LiquidationEngine,
    pub interest_model: LinearInterestRate,
    pub oracle: SimplePriceOracle,
    pub flash_loan_fee_bps: u64,
}

impl LendingMarket {
    /// Create a new lending market with default parameters.
    pub fn new() -> Self {
        Self {
            pool: LendingPool::new(),
            collateral: CollateralManager::new(),
            liquidation: LiquidationEngine::default(),
            interest_model: LinearInterestRate::default(),
            oracle: SimplePriceOracle::new(),
            flash_loan_fee_bps: DEFAULT_FLASH_LOAN_FEE_BPS,
        }
    }

    /// Initialize a reserve for a new asset.
    pub fn init_reserve(&mut self, asset: AssetId, timestamp: Timestamp) {
        self.pool.init_reserve(asset, timestamp);
    }

    /// Accrue interest on a reserve, updating borrows and the stored rate.
    pub fn accrue_interest(
        &mut self,
        asset: &AssetId,
        timestamp: Timestamp,
    ) -> DefiResult<()> {
        self.pool
            .accrue_interest(asset, &self.interest_model, timestamp)
    }

    /// Execute a flash loan against the pool.
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

    /// Get the health factor for a borrower.
    pub fn health_factor(&self, borrower: &Address) -> DefiResult<u64> {
        self.collateral
            .get_health_factor(borrower, &self.oracle)
    }

    /// Add collateral for a borrower.
    pub fn add_collateral(
        &mut self,
        borrower: Address,
        asset: AssetId,
        amount: Amount,
        ltv_ratio_bps: u64,
    ) -> DefiResult<()> {
        self.collateral
            .add_collateral(borrower, asset, amount, ltv_ratio_bps)
    }

    /// Available liquidity for an asset in the pool.
    pub fn available_liquidity(&self, asset: &AssetId) -> DefiResult<Amount> {
        self.pool.available_liquidity(asset)
    }
}

impl Default for LendingMarket {
    fn default() -> Self {
        Self::new()
    }
}

impl LendingProtocol for LendingMarket {
    /// Deposit an asset into the lending pool.
    ///
    /// Accrues interest first, then adds liquidity to the reserve.
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

    /// Borrow an asset from the pool.
    ///
    /// Accrues interest, verifies the borrower's health factor remains above
    /// the liquidation threshold after the borrow, then records the debt.
    fn borrow(
        &mut self,
        borrower: Address,
        asset: AssetId,
        amount: Amount,
        timestamp: Timestamp,
    ) -> DefiResult<()> {
        self.accrue_interest(&asset, timestamp)?;

        // Execute the borrow on the pool.
        self.pool.borrow(&asset, amount, timestamp)?;

        // Record the debt in the collateral manager.
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
        self.collateral.debts.insert(debt_key, new_debt);

        // Verify health factor is still acceptable.
        let hf = self.collateral.get_health_factor(&borrower, &self.oracle)?;
        if hf < self.liquidation.liquidation_threshold_bps {
            // Undo the borrow — position would be immediately liquidatable.
            self.pool.repay(&asset, amount, timestamp)?;
            let debt_key = (borrower, asset);
            self.collateral.debts.insert(debt_key, current_debt);
            return Err(DefiError::InsufficientCollateral {
                health_factor_bps: hf,
            });
        }

        Ok(())
    }

    /// Repay a borrowed amount.
    ///
    /// Accrues interest, then reduces the borrower's debt in both the pool
    /// and the collateral manager.
    fn repay(
        &mut self,
        borrower: Address,
        asset: AssetId,
        amount: Amount,
        timestamp: Timestamp,
    ) -> DefiResult<()> {
        self.accrue_interest(&asset, timestamp)?;

        let repaid = self.pool.repay(&asset, amount, timestamp)?;

        // Reduce debt tracking.
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

    /// Liquidate an under-collateralized position.
    ///
    /// Accrues interest on the debt asset, then delegates to the liquidation engine.
    fn liquidate(
        &mut self,
        liquidator: Address,
        borrower: Address,
        debt_asset: AssetId,
        collateral_asset: AssetId,
        amount: Amount,
        timestamp: Timestamp,
    ) -> DefiResult<Amount> {
        self.accrue_interest(&debt_asset, timestamp)?;

        let collateral_seized = self.liquidation.liquidate(
            liquidator,
            &borrower,
            &debt_asset,
            &collateral_asset,
            amount,
            &mut self.collateral,
            &self.oracle,
        )?;

        // The liquidation engine already updated collateral manager debts.
        // Update the pool's borrow tracking to reflect the repaid amount.
        self.pool.repay(&debt_asset, amount, timestamp)?;

        Ok(collateral_seized)
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
        bytes[0] = n;
        Address::from_bytes(bytes)
    }

    fn setup_market() -> LendingMarket {
        let mut market = LendingMarket::new();
        market.init_reserve(asset("USDC"), ts(0));
        market.init_reserve(asset("ETH"), ts(0));
        market.oracle.set_price(asset("USDC"), Price::from_integer(1));
        market.oracle.set_price(asset("ETH"), Price::from_integer(2000));
        market
    }

    #[test]
    fn test_deposit_and_borrow_flow() {
        let mut market = setup_market();

        // Supplier deposits USDC.
        market
            .deposit(addr(1), asset("USDC"), Amount::new(100_000), ts(1))
            .unwrap();
        assert_eq!(
            market.pool.available_liquidity(&asset("USDC")).unwrap(),
            Amount::new(100_000)
        );

        // Borrower posts ETH collateral (1 ETH = 2000 USDC, LTV 75%).
        market
            .add_collateral(addr(2), asset("ETH"), Amount::new(1), 7500)
            .unwrap();

        // Borrower borrows USDC.
        // Collateral value: 1 * 2000 = 2000, weighted = 2000 * 7500 / 10000 = 1500
        // Debt value: 1000 * 1 = 1000
        // HF = 1500 * 10000 / 1000... actually let's check the HF formula:
        // hf = weighted_collateral_value / total_debt_value
        // = (1 * 2000 * 7500) / (1000 * 1)  = 15_000_000 / 1000 = 15_000
        // 15_000 > 10_000 => healthy
        market
            .borrow(addr(2), asset("USDC"), Amount::new(1_000), ts(2))
            .unwrap();

        assert_eq!(
            market.collateral.debts[&(addr(2), asset("USDC"))],
            Amount::new(1_000)
        );
        assert_eq!(
            market.pool.available_liquidity(&asset("USDC")).unwrap(),
            Amount::new(99_000)
        );
    }

    #[test]
    fn test_borrow_exceeds_health_factor_fails() {
        let mut market = setup_market();

        market
            .deposit(addr(1), asset("USDC"), Amount::new(100_000), ts(1))
            .unwrap();

        // Post small collateral: 1 ETH = 2000 USDC value, LTV 75%.
        market
            .add_collateral(addr(2), asset("ETH"), Amount::new(1), 7500)
            .unwrap();

        // Try to borrow more than collateral allows.
        // Max safe borrow: weighted_collateral / threshold = (1*2000*7500) / 10_000 = 1500
        // Borrowing 2000 USDC: HF = 15_000_000 / 2000 = 7500 < 10_000 => rejected
        let result = market.borrow(addr(2), asset("USDC"), Amount::new(2_000), ts(2));
        assert!(result.is_err());

        // Pool borrow should have been rolled back.
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
            .unwrap();
        market
            .add_collateral(addr(2), asset("ETH"), Amount::new(1), 7500)
            .unwrap();
        market
            .borrow(addr(2), asset("USDC"), Amount::new(1_000), ts(2))
            .unwrap();

        // Repay half.
        market
            .repay(addr(2), asset("USDC"), Amount::new(500), ts(3))
            .unwrap();

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
            .unwrap();
        market
            .add_collateral(addr(2), asset("ETH"), Amount::new(1), 7500)
            .unwrap();
        market
            .borrow(addr(2), asset("USDC"), Amount::new(1_000), ts(2))
            .unwrap();

        // Repay everything.
        market
            .repay(addr(2), asset("USDC"), Amount::new(1_000), ts(3))
            .unwrap();

        // Debt entry should be removed.
        assert!(!market.collateral.debts.contains_key(&(addr(2), asset("USDC"))));
        assert_eq!(
            market.pool.reserves[&asset("USDC")].total_borrows,
            Amount::ZERO
        );
    }

    #[test]
    fn test_liquidation_through_market() {
        let mut market = setup_market();

        // Deposit liquidity.
        market
            .deposit(addr(1), asset("USDC"), Amount::new(100_000), ts(1))
            .unwrap();

        // Borrower posts ETH collateral and borrows close to the limit.
        market
            .add_collateral(addr(2), asset("ETH"), Amount::new(500), 7500)
            .unwrap();
        // weighted_collateral = 500 * 2000 * 7500 = 7_500_000_000
        // Borrow 1000 USDC: hf = 7_500_000_000 / 1000 = 7_500_000 >> 10_000, healthy.
        market
            .borrow(addr(2), asset("USDC"), Amount::new(1_000), ts(2))
            .unwrap();

        // Price of ETH crashes — now 1 ETH = 1 USDC (via oracle update).
        market.oracle.set_price(asset("ETH"), Price::from_integer(1));
        // Now: weighted_collateral = 500 * 1 * 7500 = 3_750_000
        // debt = 1000 * 1 = 1000
        // hf = 3_750_000 / 1000 = 3750 < 10_000 => liquidatable

        // Liquidator liquidates.
        let seized = market
            .liquidate(
                addr(3),
                addr(2),
                asset("USDC"),
                asset("ETH"),
                Amount::new(200),
                ts(3),
            )
            .unwrap();

        // Reward: 200 USDC * 1 (price) * 1.05 (bonus) / 1 (ETH price) = 210 ETH
        assert_eq!(seized, Amount::new(210));
        // Remaining debt in collateral manager: 1000 - 200 = 800
        assert_eq!(
            market.collateral.debts[&(addr(2), asset("USDC"))],
            Amount::new(800)
        );
    }

    #[test]
    fn test_interest_accrual_over_time() {
        let mut market = setup_market();

        // Deposit and borrow.
        market
            .deposit(addr(1), asset("USDC"), Amount::new(100_000), ts(0))
            .unwrap();
        market
            .add_collateral(addr(2), asset("ETH"), Amount::new(100), 7500)
            .unwrap();
        market
            .borrow(addr(2), asset("USDC"), Amount::new(50_000), ts(0))
            .unwrap();

        let borrows_before = market.pool.reserves[&asset("USDC")].total_borrows;
        assert_eq!(borrows_before, Amount::new(50_000));

        // Advance time by 1 year (365 * 24 * 3600 = 31_536_000 seconds).
        // Utilization = 50_000 / 100_000 = 50% = 5000 bps
        // Below optimal (8000): rate = 200 + (5000 * 400 / 8000) = 200 + 250 = 450 bps
        // Interest = 50_000 * 450 * 31_536_000 / (31_536_000 * 10_000) = 50_000 * 450 / 10_000 = 2250
        let one_year = 365 * 24 * 3600;
        market.accrue_interest(&asset("USDC"), ts(one_year)).unwrap();

        let borrows_after = market.pool.reserves[&asset("USDC")].total_borrows;
        assert_eq!(borrows_after, Amount::new(50_000 + 2_250));
    }

    #[test]
    fn test_flash_loan_through_market() {
        let mut market = setup_market();
        market
            .deposit(addr(1), asset("USDC"), Amount::new(100_000), ts(1))
            .unwrap();

        let fee = market
            .flash_loan(&asset("USDC"), Amount::new(50_000), ts(2), |_borrowed| {
                // fee = 50_000 * 9 / 10_000 = 45
                Ok(Amount::new(50_045))
            })
            .unwrap();

        assert_eq!(fee, Amount::new(45));
        // Pool gained the fee: 100_000 + 45 = 100_045
        assert_eq!(
            market.pool.reserves[&asset("USDC")].total_deposits,
            Amount::new(100_045)
        );
    }

    #[test]
    fn test_health_factor_no_debt() {
        let market = setup_market();
        assert_eq!(market.health_factor(&addr(1)).unwrap(), u64::MAX);
    }
}
