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
