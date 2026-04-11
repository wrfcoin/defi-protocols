//! Parametric weather insurance protocol with oracle-triggered payouts.
//!
//! This crate mirrors the semantics of `ParametricInsurance.sol` (smart-contracts #98)
//! in pure Rust, integrated with the WRFCoin DeFi type system.
//!
//! ## Architecture
//!
//! - [`WeatherTrigger`]: enum describing conditions that activate a payout
//! - [`ParametricPolicy`]: a purchased insurance policy
//! - [`PolicyStore`]: registry of all active policies
//! - [`InsuranceVault`]: ERC-4626-style underwriter capital pool
//! - [`SettlementEngine`]: evaluates triggers against oracle readings and pays out
//! - [`PremiumPricing`]: actuarial pricing model (historical frequency + loading factor)

pub mod error;
pub mod oracle;
pub mod policy;
pub mod pricing;
pub mod settlement;
pub mod vault;

pub use error::{InsuranceError, InsuranceResult};
pub use oracle::{GeoCoordinate, WeatherReading};
pub use policy::{
    CompositeLogic, ObservationWindow, ParametricPolicy, PolicyId, PolicyState, PolicyStore,
    WeatherTrigger,
};
pub use pricing::{HistoricalRecord, PremiumPricing, TriggerTypeKey};
pub use settlement::{SettlementEngine, SettlementOutcome};
pub use vault::{InsuranceVault, RiskLimits, VaultConfig, VaultShares};
