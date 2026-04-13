use crate::error::DefiResult;
use crate::types::{
    Address, Amount, AssetId, LiquidationEvent, PoolId, Price, Timestamp, UtilitySignalSnapshot,
};

/// Core trait for lending protocol implementations.
pub trait LendingProtocol {
    /// Deposit an asset into the lending pool.
    fn deposit(
        &mut self,
        depositor: Address,
        asset: AssetId,
        amount: Amount,
        timestamp: Timestamp,
    ) -> DefiResult<()>;

    /// Borrow an asset from the lending pool using existing collateral.
    fn borrow(
        &mut self,
        borrower: Address,
        asset: AssetId,
        amount: Amount,
        timestamp: Timestamp,
    ) -> DefiResult<()>;

    /// Repay a borrowed amount.
    fn repay(
        &mut self,
        borrower: Address,
        asset: AssetId,
        amount: Amount,
        timestamp: Timestamp,
    ) -> DefiResult<()>;

    /// Liquidate an under-collateralized position.
    fn liquidate(
        &mut self,
        liquidator: Address,
        borrower: Address,
        debt_asset: AssetId,
        collateral_asset: AssetId,
        amount: Amount,
        timestamp: Timestamp,
    ) -> DefiResult<LiquidationEvent>;
}

/// Trait for automated market maker (AMM) liquidity pools.
pub trait AMMPool {
    /// Add liquidity to the pool. Returns the number of LP tokens minted.
    fn add_liquidity(
        &mut self,
        provider: Address,
        amount_a: Amount,
        amount_b: Amount,
        timestamp: Timestamp,
    ) -> DefiResult<Amount>;

    /// Swap one token for another. Returns the output amount.
    fn swap(
        &mut self,
        trader: Address,
        amount_in: Amount,
        asset_in: AssetId,
        min_amount_out: Amount,
        timestamp: Timestamp,
    ) -> DefiResult<Amount>;

    /// Remove liquidity by burning LP tokens. Returns (amount_a, amount_b).
    fn remove_liquidity(
        &mut self,
        provider: Address,
        lp_amount: Amount,
        timestamp: Timestamp,
    ) -> DefiResult<(Amount, Amount)>;

    /// Get the current reserves in the pool.
    fn get_reserves(&self) -> (Amount, Amount);
}

/// Trait for yield farming protocols.
pub trait YieldFarm {
    /// Stake LP tokens into the farm.
    fn stake(
        &mut self,
        staker: Address,
        pool_id: PoolId,
        amount: Amount,
        timestamp: Timestamp,
    ) -> DefiResult<()>;

    /// Harvest accumulated rewards.
    fn harvest(
        &mut self,
        staker: Address,
        pool_id: PoolId,
        timestamp: Timestamp,
    ) -> DefiResult<Amount>;

    /// Unstake LP tokens from the farm.
    fn unstake(
        &mut self,
        staker: Address,
        pool_id: PoolId,
        amount: Amount,
        timestamp: Timestamp,
    ) -> DefiResult<Amount>;

    /// Get pending (unharvested) rewards for a staker in a pool.
    fn pending_rewards(
        &self,
        staker: Address,
        pool_id: PoolId,
        timestamp: Timestamp,
    ) -> DefiResult<Amount>;
}

/// Trait for price oracle implementations.
pub trait PriceOracle {
    /// Get the price of a single asset (denominated in the base currency).
    fn get_price(&self, asset: &AssetId) -> DefiResult<Price>;

    /// Get prices for multiple assets at once.
    fn get_prices(&self, assets: &[AssetId]) -> DefiResult<Vec<(AssetId, Price)>> {
        assets
            .iter()
            .map(|a| {
                let price = self.get_price(a)?;
                Ok((a.clone(), price))
            })
            .collect()
    }
}

/// Source of verified utility telemetry for reward weighting hooks.
pub trait UtilitySignalSource {
    /// Return the latest verified utility signal snapshot for a pool.
    fn utility_signal_for_pool(
        &self,
        pool_id: PoolId,
        timestamp: Timestamp,
    ) -> DefiResult<UtilitySignalSnapshot>;
}
