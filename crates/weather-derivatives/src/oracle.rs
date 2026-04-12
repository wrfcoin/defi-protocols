//! Weather oracle trait and in-memory test implementation.

use crate::error::{DerivativesError, DerivativesResult};
use crate::index::GeographicRegion;
use std::collections::HashMap;

/// An oracle-attested weather reading for a region at a block height.
/// The `value_scaled` field is scaled by 1_000_000.
#[derive(Debug, Clone)]
pub struct OracleAttestment {
    pub region: GeographicRegion,
    pub block: u64,
    /// Reading value scaled by 1_000_000 (e.g., temperature in °F, precip in mm, wind in m/s).
    pub value_scaled: i64,
}

/// Trait for weather oracle providers.
pub trait WeatherOracle {
    /// Returns the attested reading for a region at (or aggregated over) a given block.
    fn get_reading(&self, region: &GeographicRegion, block: u64) -> DerivativesResult<i64>;
}

/// Simple in-memory oracle for unit tests.
#[derive(Debug, Default)]
pub struct InMemoryOracle {
    readings: HashMap<(String, u64), i64>,
}

impl InMemoryOracle {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a reading for a region + block.
    pub fn set(&mut self, region: &GeographicRegion, block: u64, value_scaled: i64) {
        self.readings
            .insert((region.0.clone(), block), value_scaled);
    }
}

impl WeatherOracle for InMemoryOracle {
    fn get_reading(&self, region: &GeographicRegion, block: u64) -> DerivativesResult<i64> {
        self.readings
            .get(&(region.0.clone(), block))
            .copied()
            .ok_or_else(|| DerivativesError::OracleReadingUnavailable {
                region: region.0.clone(),
                block,
            })
    }
}
