use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use wrfcoin_defi_common::error::{DefiError, DefiResult};
use wrfcoin_defi_common::traits::PriceOracle;
use wrfcoin_defi_common::types::{AssetId, Price, Timestamp};

/// A point-in-time oracle price update.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PriceSnapshot {
    pub price: Price,
    pub updated_at: Timestamp,
}

/// A simple in-memory price oracle for development and testing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimplePriceOracle {
    prices: HashMap<AssetId, PriceSnapshot>,
}

impl SimplePriceOracle {
    pub fn new() -> Self {
        Self {
            prices: HashMap::new(),
        }
    }

    /// Set or update the price for an asset with a timestamp.
    pub fn set_price_at(&mut self, asset: AssetId, price: Price, updated_at: Timestamp) {
        self.prices.insert(asset, PriceSnapshot { price, updated_at });
    }

    /// Backwards-compatible setter for tests that do not care about freshness.
    pub fn set_price(&mut self, asset: AssetId, price: Price) {
        self.set_price_at(asset, price, Timestamp::new(0));
    }

    pub fn get_snapshot(&self, asset: &AssetId) -> DefiResult<PriceSnapshot> {
        self.prices
            .get(asset)
            .copied()
            .ok_or_else(|| DefiError::OracleError {
                message: format!("no price available for asset {}", asset),
            })
    }

    pub fn ensure_fresh(
        &self,
        assets: &[AssetId],
        now: Timestamp,
        max_age_secs: u64,
    ) -> DefiResult<()> {
        for asset in assets {
            let snapshot = self.get_snapshot(asset)?;
            let age = now.elapsed_since(snapshot.updated_at);
            if age > max_age_secs {
                return Err(DefiError::OracleError {
                    message: format!(
                        "stale oracle price for asset {}: age {}s exceeds {}s",
                        asset, age, max_age_secs
                    ),
                });
            }
        }
        Ok(())
    }

    pub fn remove_price(&mut self, asset: &AssetId) {
        self.prices.remove(asset);
    }

    pub fn has_price(&self, asset: &AssetId) -> bool {
        self.prices.contains_key(asset)
    }
}

impl PriceOracle for SimplePriceOracle {
    fn get_price(&self, asset: &AssetId) -> DefiResult<Price> {
        self.get_snapshot(asset).map(|snapshot| snapshot.price)
    }
}

impl Default for SimplePriceOracle {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_set_and_get_price() {
        let mut oracle = SimplePriceOracle::new();
        let wrf = AssetId::new("WRF");
        oracle.set_price_at(wrf.clone(), Price::new(2, 1), Timestamp::new(42));
        let snapshot = oracle.get_snapshot(&wrf).expect("snapshot should exist after set_price_at");
        assert_eq!(snapshot.price, Price::new(2, 1));
        assert_eq!(snapshot.updated_at, Timestamp::new(42));
    }

    #[test]
    fn test_missing_price() {
        let oracle = SimplePriceOracle::new();
        let result = oracle.get_price(&AssetId::new("UNKNOWN"));
        assert!(result.is_err());
    }

    #[test]
    fn test_remove_price() {
        let mut oracle = SimplePriceOracle::new();
        let wrf = AssetId::new("WRF");
        oracle.set_price_at(wrf.clone(), Price::new(1, 1), Timestamp::new(1));
        assert!(oracle.has_price(&wrf));
        oracle.remove_price(&wrf);
        assert!(!oracle.has_price(&wrf));
    }

    #[test]
    fn test_get_prices_batch() {
        let mut oracle = SimplePriceOracle::new();
        oracle.set_price_at(AssetId::new("A"), Price::new(1, 1), Timestamp::new(1));
        oracle.set_price_at(AssetId::new("B"), Price::new(2, 1), Timestamp::new(1));
        let prices = oracle.get_prices(&[AssetId::new("A"), AssetId::new("B")]).expect("batch price lookup should succeed for known assets");
        assert_eq!(prices.len(), 2);
    }

    #[test]
    fn test_ensure_fresh_rejects_stale_price() {
        let mut oracle = SimplePriceOracle::new();
        oracle.set_price_at(AssetId::new("A"), Price::new(1, 1), Timestamp::new(5));
        let result = oracle.ensure_fresh(&[AssetId::new("A")], Timestamp::new(20), 10);
        assert!(matches!(result, Err(DefiError::OracleError { .. })));
    }
}
