pub mod collateral;
pub mod flash_loan;
pub mod interest;
pub mod liquidation;
pub mod market;
pub mod oracle;
pub mod pool;
pub mod wire;

pub use collateral::{CollateralConfig, CollateralManager, CollateralPosition};
pub use flash_loan::{execute_flash_loan, DEFAULT_FLASH_LOAN_FEE_BPS};
pub use interest::{InterestRateModel, LinearInterestRate};
pub use liquidation::{LiquidationEngine, LiquidationOutcome};
pub use market::LendingMarket;
pub use oracle::{PriceSnapshot, SimplePriceOracle};
pub use pool::{LendingPool, ReserveData};
pub use wire::{
    DecimalString, LendingAccountId, LendingWirePositionV0, RiskBps, WireSchemaError,
    LENDING_INTERFACE_V0,
};
pub use wrfcoin_defi_common::types::LiquidationEvent;
