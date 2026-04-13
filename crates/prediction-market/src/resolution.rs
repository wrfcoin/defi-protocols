use serde::{Deserialize, Serialize};

use crate::error::{PredictionError, PredictionResult};
use crate::market::{ForecastMarket, MarketKind, MarketStatus};

/// Oracle-attested verification result for a forecast.
/// In production this would be a cross-chain attestation or signed receipt.
/// In tests it is constructed directly (mock stub — no cross-repo import required).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForecastVerificationReceipt {
    /// The forecast manifest hash this receipt attests to.
    pub forecast_manifest_hash: [u8; 32],
    /// Block height at which the oracle verified this result.
    pub verified_at_height: u64,
    /// Accuracy metric for the forecast (e.g., RMSE in °C, or mm, or m/s).
    /// Lower is better for error metrics (RMSE, MAE); the market threshold
    /// expresses the direction explicitly.
    pub accuracy_score: f64,
}

/// Result of resolving a market.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolutionResult {
    pub market_id: u64,
    /// Index of the winning outcome (Binary: 0=Yes, 1=No; Scalar: always 0).
    pub winning_outcome: usize,
    /// Raw accuracy score from the oracle.
    pub accuracy_score: f64,
}

/// Resolves forecast markets when their `resolution_height` is reached.
pub struct ResolutionEngine;

impl ResolutionEngine {
    /// Attempt to resolve a market.
    ///
    /// Rules:
    /// - Returns `ResolutionTooEarly` if `current_height < market.resolution_height`.
    /// - Returns `AlreadyResolved` if the market is not `Open`.
    /// - Binary: resolves Yes (outcome 0) if `accuracy_score < threshold`; No (outcome 1) otherwise.
    /// - Scalar: always resolves outcome 0; actual payout is proportional to `accuracy_score`.
    pub fn resolve(
        market: &mut ForecastMarket,
        receipt: &ForecastVerificationReceipt,
        current_height: u64,
    ) -> PredictionResult<ResolutionResult> {
        // Guard: too early?
        if current_height < market.resolution_height {
            return Err(PredictionError::ResolutionTooEarly {
                current_height,
                resolution_height: market.resolution_height,
            });
        }

        // Guard: already resolved?
        if !market.is_open() {
            return Err(PredictionError::AlreadyResolved {
                market_id: market.id,
            });
        }

        let accuracy_score = receipt.accuracy_score;

        let winning_outcome = match market.kind {
            MarketKind::Binary => {
                // Outcome 0 = Yes bin, which carries the threshold.
                let threshold =
                    market.outcome_bins[0]
                        .threshold
                        .ok_or_else(|| PredictionError::LmsrError {
                            reason: "binary market Yes bin missing threshold".into(),
                        })?;
                // "Yes" wins when the error metric is below the threshold.
                if accuracy_score < threshold {
                    0 // Yes
                } else {
                    1 // No
                }
            }
            MarketKind::Scalar => 0, // single outcome; payout uses raw score
        };

        market.status = MarketStatus::Resolved {
            winning_outcome,
            accuracy_score,
        };

        Ok(ResolutionResult {
            market_id: market.id,
            winning_outcome,
            accuracy_score,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::market::{ForecastMarket, WeatherMetric};
    use wrfcoin_defi_common::types::Address;

    fn make_binary_market(resolution_height: u64, threshold: f64) -> ForecastMarket {
        ForecastMarket::new_binary(
            1,
            [0u8; 32],
            "North-Atlantic",
            WeatherMetric::Temperature,
            resolution_height,
            threshold,
            100.0,
            30,
            Address::zero(),
        )
    }

    fn make_receipt(accuracy_score: f64) -> ForecastVerificationReceipt {
        ForecastVerificationReceipt {
            forecast_manifest_hash: [0u8; 32],
            verified_at_height: 100,
            accuracy_score,
        }
    }

    #[test]
    fn test_binary_yes_resolution() {
        let mut market = make_binary_market(100, 2.0);
        let receipt = make_receipt(1.5); // RMSE 1.5 < threshold 2.0 → Yes
        let result = ResolutionEngine::resolve(&mut market, &receipt, 100).unwrap();
        assert_eq!(
            result.winning_outcome, 0,
            "RMSE < threshold should resolve Yes"
        );
    }

    #[test]
    fn test_binary_no_resolution() {
        let mut market = make_binary_market(100, 2.0);
        let receipt = make_receipt(2.5); // RMSE 2.5 >= threshold 2.0 → No
        let result = ResolutionEngine::resolve(&mut market, &receipt, 100).unwrap();
        assert_eq!(
            result.winning_outcome, 1,
            "RMSE >= threshold should resolve No"
        );
    }

    #[test]
    fn test_resolution_too_early() {
        let mut market = make_binary_market(200, 2.0);
        let receipt = make_receipt(1.5);
        let err = ResolutionEngine::resolve(&mut market, &receipt, 100).unwrap_err();
        assert!(
            matches!(err, PredictionError::ResolutionTooEarly { .. }),
            "should error before resolution_height"
        );
    }

    #[test]
    fn test_double_resolution_guard() {
        let mut market = make_binary_market(100, 2.0);
        let receipt = make_receipt(1.5);
        ResolutionEngine::resolve(&mut market, &receipt, 100).unwrap();
        let err = ResolutionEngine::resolve(&mut market, &receipt, 100).unwrap_err();
        assert!(
            matches!(err, PredictionError::AlreadyResolved { .. }),
            "double-resolve should error"
        );
    }

    #[test]
    fn test_scalar_market_resolution() {
        let mut market = ForecastMarket::new_scalar(
            2,
            [0u8; 32],
            "Pacific",
            WeatherMetric::Precipitation,
            50,
            100.0,
            30,
            Address::zero(),
        );
        let receipt = make_receipt(0.75);
        let result = ResolutionEngine::resolve(&mut market, &receipt, 50).unwrap();
        assert_eq!(result.winning_outcome, 0);
        assert!((result.accuracy_score - 0.75).abs() < 1e-9);
    }
}
