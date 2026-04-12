use serde::{Deserialize, Serialize};
use wrfcoin_defi_common::types::{Address, Amount};

/// The weather metric this market is forecasting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum WeatherMetric {
    /// Temperature, measured in degrees Celsius.
    Temperature,
    /// Precipitation, measured in millimetres.
    Precipitation,
    /// Wind speed, measured in metres per second.
    Wind,
}

/// A single discrete outcome slot in the market (used for labelling).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OutcomeBin {
    /// Human-readable label, e.g. "Yes", "No", or "0–2°C".
    pub label: String,
    /// For binary markets: the accuracy threshold this bin represents.
    /// For scalar markets: not used for resolution logic (None).
    pub threshold: Option<f64>,
}

impl OutcomeBin {
    pub fn yes(threshold: f64) -> Self {
        Self {
            label: "Yes".to_string(),
            threshold: Some(threshold),
        }
    }

    pub fn no() -> Self {
        Self {
            label: "No".to_string(),
            threshold: None,
        }
    }

    pub fn scalar(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            threshold: None,
        }
    }
}

/// Whether the market resolves to a discrete winner or a continuous scalar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MarketKind {
    /// Two outcomes: "Yes" (index 0) and "No" (index 1).
    /// Resolves Yes if the oracle accuracy metric beats the threshold.
    Binary,
    /// Single continuous outcome; payout is proportional to the accuracy score.
    Scalar,
}

/// Current lifecycle state of a market.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum MarketStatus {
    /// Open for trading.
    Open,
    /// Resolved — stores the winning outcome index (for Binary) or scalar score.
    Resolved {
        /// Index of the winning outcome bin (Binary), or 0 for Scalar.
        winning_outcome: usize,
        /// Accuracy score provided by the oracle (RMSE or similar metric).
        accuracy_score: f64,
    },
}

/// A prediction market for weather forecast accuracy.
///
/// Binary example: "Will RMSE < 2°C for the North-Atlantic region?"
///   outcome_bins = [OutcomeBin::yes(2.0), OutcomeBin::no()]
///   A resolution oracle result of RMSE=1.5 resolves to outcome 0 (Yes).
///
/// Scalar example: payout proportional to accuracy_score (e.g. 1/RMSE normalised to [0,1]).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForecastMarket {
    pub id: u64,
    /// BLAKE3 hash of the forecast manifest this market is tied to.
    pub forecast_manifest_hash: [u8; 32],
    /// Geographic region identifier.
    pub oracle_region: String,
    /// The weather metric being predicted.
    pub metric: WeatherMetric,
    /// Block height at which the market can be resolved.
    pub resolution_height: u64,
    /// Outcome bins — at least 2 for Binary, exactly 1 for Scalar.
    pub outcome_bins: Vec<OutcomeBin>,
    /// Binary or Scalar.
    pub kind: MarketKind,
    /// LMSR liquidity parameter `b`.  Larger `b` = more liquidity, more initial funding required.
    pub lmsr_b: f64,
    /// Current share quantities per outcome.  `q[i]` = total shares sold for outcome i.
    pub q: Vec<f64>,
    /// Total WRF-base-units staked into this market (used for payout pool).
    pub total_stake: Amount,
    /// Protocol fee in basis points (e.g. 30 = 0.3%).
    pub fee_bps: u64,
    /// Creator / admin address.
    pub creator: Address,
    /// Current status.
    pub status: MarketStatus,
}

impl ForecastMarket {
    /// Create a new Binary market.
    #[allow(clippy::too_many_arguments)]
    pub fn new_binary(
        id: u64,
        forecast_manifest_hash: [u8; 32],
        oracle_region: impl Into<String>,
        metric: WeatherMetric,
        resolution_height: u64,
        threshold: f64,
        lmsr_b: f64,
        fee_bps: u64,
        creator: Address,
    ) -> Self {
        let outcome_bins = vec![OutcomeBin::yes(threshold), OutcomeBin::no()];
        let q = vec![0.0_f64; 2];
        Self {
            id,
            forecast_manifest_hash,
            oracle_region: oracle_region.into(),
            metric,
            resolution_height,
            outcome_bins,
            kind: MarketKind::Binary,
            lmsr_b,
            q,
            total_stake: Amount::ZERO,
            fee_bps,
            creator,
            status: MarketStatus::Open,
        }
    }

    /// Create a new Scalar market.
    #[allow(clippy::too_many_arguments)]
    pub fn new_scalar(
        id: u64,
        forecast_manifest_hash: [u8; 32],
        oracle_region: impl Into<String>,
        metric: WeatherMetric,
        resolution_height: u64,
        lmsr_b: f64,
        fee_bps: u64,
        creator: Address,
    ) -> Self {
        let outcome_bins = vec![OutcomeBin::scalar("Accuracy")];
        let q = vec![0.0_f64; 1];
        Self {
            id,
            forecast_manifest_hash,
            oracle_region: oracle_region.into(),
            metric,
            resolution_height,
            outcome_bins,
            kind: MarketKind::Scalar,
            lmsr_b,
            q,
            total_stake: Amount::ZERO,
            fee_bps,
            creator,
            status: MarketStatus::Open,
        }
    }

    pub fn is_open(&self) -> bool {
        self.status == MarketStatus::Open
    }

    pub fn num_outcomes(&self) -> usize {
        self.outcome_bins.len()
    }
}
