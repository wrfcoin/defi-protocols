//! # wrfcoin-prediction-market
//!
//! LMSR-based prediction markets for WRFCoin weather forecast accuracy.
//!
//! ## Architecture
//!
//! - [`market`]: `ForecastMarket`, `WeatherMetric`, `OutcomeBin`, `MarketStatus`
//! - [`lmsr`]: Cost function `C(q) = b·ln(Σ exp(qᵢ/b))`, buy/sell pricing (f64)
//! - [`position`]: `UserPosition`, `PositionLedger`
//! - [`resolution`]: `ResolutionEngine` — resolves markets at `resolution_height`
//! - [`payout`]: `PayoutEngine` — pro-rata payout computation with fee deduction
//! - [`engine`]: `PredictionMarketEngine` — top-level coordinator
//! - [`error`]: `PredictionError`, `PredictionResult`

pub mod engine;
pub mod error;
pub mod lmsr;
pub mod market;
pub mod payout;
pub mod position;
pub mod resolution;

pub use engine::PredictionMarketEngine;
pub use error::{PredictionError, PredictionResult};
pub use market::{ForecastMarket, MarketKind, MarketStatus, OutcomeBin, WeatherMetric};
pub use payout::{PayoutEngine, PayoutResult, UserPayout};
pub use position::{PositionLedger, UserPosition};
pub use resolution::{ForecastVerificationReceipt, ResolutionEngine, ResolutionResult};
