//! Weather derivatives engine for the WRFCoin DeFi platform.
//!
//! Provides:
//! - [`WeatherIndex`] enum: TemperatureIndex (HDD/CDD), PrecipitationIndex, WindSpeedIndex
//! - [`WeatherFuture`] / [`FuturesBook`]: oracle-settled futures
//! - [`WeatherOption`] / [`OptionsEngine`]: European-style options with Black-76 pricing
//! - [`MarginEngine`]: initial/maintenance margin, mark-to-market, liquidation hook
//! - [`SettlementEngine`]: automatic settlement against oracle-attested readings
//!
//! # Black-76 note
//! Option pricing uses f64 math (standard library only) with a polynomial approximation
//! for the standard normal CDF (Hart's method, max error ~1.5e-7). This is documented
//! as a non-consensus path — pricing is advisory only and does not affect on-chain
//! settlement amounts, which are integer arithmetic.

pub mod error;
pub mod index;
pub mod futures;
pub mod options;
pub mod margin;
pub mod settlement;
pub mod oracle;

pub use error::{DerivativesError, DerivativesResult};
pub use index::{
    GeographicRegion, ObservationWindow, TemperatureIndexKind, WeatherIndex,
};
pub use futures::{FuturesBook, FutureId, FuturePosition, FutureStatus, WeatherFuture};
pub use options::{
    OptionType, OptionsEngine, WeatherOption, OptionId, OptionStatus, Black76Params,
};
pub use margin::{MarginAccount, MarginEngine, MarginConfig, LiquidationHook};
pub use settlement::{OracleReading, SettlementEngine, SettlementOutcome};
pub use oracle::WeatherOracle;
