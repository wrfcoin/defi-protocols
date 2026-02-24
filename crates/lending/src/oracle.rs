use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use wrfcoin_defi_common::error::{DefiError, DefiResult};
use wrfcoin_defi_common::traits::PriceOracle;
use wrfcoin_defi_common::types::{AssetId, Price};

/// A simple in-memory price oracle for development and testing.
///
/// Prices are set manually and do not update automatically.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimplePriceOracle {
    prices: HashMap<AssetId, Price>,
}

impl SimplePriceOracle {
    pub fn new() -> Self {
        Self {
            prices: HashMap::new(),
        }
    }

    /// Set or update the price for an asset.
    pub fn set_price(&mut self, asset: AssetId, price: Price) {
        self.prices.insert(asset, price);
    }

    /// Remove the price entry for an asset.
    pub fn remove_price(&mut self, asset: &AssetId) {
        self.prices.remove(asset);
    }

    /// Check whether a price is available for the given asset.
    pub fn has_price(&self, asset: &AssetId) -> bool {
        self.prices.contains_key(asset)
    }
}

impl PriceOracle for SimplePriceOracle {
    fn get_price(&self, asset: &AssetId) -> DefiResult<Price> {
        self.prices
            .get(asset)
            .copied()
            .ok_or_else(|| DefiError::OracleError {
                message: format!("no price available for asset {}", asset),
            })
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
        oracle.set_price(wrf.clone(), Price::new(2, 1));
        let price = oracle.get_price(&wrf).unwrap();
        assert_eq!(price, Price::new(2, 1));
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
        oracle.set_price(wrf.clone(), Price::new(1, 1));
        assert!(oracle.has_price(&wrf));
        oracle.remove_price(&wrf);
        assert!(!oracle.has_price(&wrf));
    }

    #[test]
    fn test_get_prices_batch() {
        let mut oracle = SimplePriceOracle::new();
        oracle.set_price(AssetId::new("A"), Price::new(1, 1));
        oracle.set_price(AssetId::new("B"), Price::new(2, 1));
        let prices = oracle.get_prices(&[AssetId::new("A"), AssetId::new("B")]).unwrap();
        assert_eq!(prices.len(), 2);
    }
}
