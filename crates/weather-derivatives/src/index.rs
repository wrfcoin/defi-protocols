//! Weather index types used as underlyings for futures and options.

use serde::{Deserialize, Serialize};

/// Identifies a geographic region for weather observation.
/// Uses a simple string identifier (e.g., "US-NYC", "EU-LONDON", "APAC-TOKYO").
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct GeographicRegion(pub String);

impl GeographicRegion {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for GeographicRegion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A block-height range defining when weather data is observed.
/// Both bounds are inclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservationWindow {
    /// First block of the observation period.
    pub start_block: u64,
    /// Last block of the observation period (inclusive).
    pub end_block: u64,
}

impl ObservationWindow {
    pub fn new(start_block: u64, end_block: u64) -> Self {
        assert!(
            end_block >= start_block,
            "end_block must be >= start_block"
        );
        Self {
            start_block,
            end_block,
        }
    }

    pub fn contains(&self, block: u64) -> bool {
        block >= self.start_block && block <= self.end_block
    }

    pub fn is_closed_at(&self, block: u64) -> bool {
        block > self.end_block
    }
}

/// Discriminant for temperature index type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TemperatureIndexKind {
    /// Heating Degree Days: max(0, base_temp_f - observed_temp_f) per day, summed.
    HDD,
    /// Cooling Degree Days: max(0, observed_temp_f - base_temp_f) per day, summed.
    CDD,
}

/// Weather index underlying for a derivative contract.
///
/// All fixed-point values use a scale of 1_000_000 (1e6) to represent decimals
/// without floating point.  E.g., 65.0°F is stored as 65_000_000.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum WeatherIndex {
    /// Heating/Cooling Degree Days computed over the observation window.
    ///
    /// `base_temp_scaled` is in tenths of a degree Fahrenheit × 1_000_000.
    /// The oracle reports `observed_temp_scaled` in the same unit.
    Temperature {
        kind: TemperatureIndexKind,
        /// Base temperature for HDD/CDD computation, scaled by 1_000_000.
        /// Example: 65.0°F → 65_000_000.
        base_temp_scaled: i64,
        region: GeographicRegion,
        window: ObservationWindow,
    },

    /// Cumulative precipitation in millimetres × 1_000_000 over the window.
    Precipitation {
        region: GeographicRegion,
        window: ObservationWindow,
    },

    /// Average wind speed in m/s × 1_000_000 over the window.
    WindSpeed {
        region: GeographicRegion,
        window: ObservationWindow,
    },
}

impl WeatherIndex {
    pub fn region(&self) -> &GeographicRegion {
        match self {
            WeatherIndex::Temperature { region, .. } => region,
            WeatherIndex::Precipitation { region, .. } => region,
            WeatherIndex::WindSpeed { region, .. } => region,
        }
    }

    pub fn window(&self) -> ObservationWindow {
        match self {
            WeatherIndex::Temperature { window, .. } => *window,
            WeatherIndex::Precipitation { window, .. } => *window,
            WeatherIndex::WindSpeed { window, .. } => *window,
        }
    }

    /// Compute the index value from an oracle-attested reading (already scaled ×1_000_000).
    ///
    /// For Temperature: returns HDD or CDD = max(0, ±(base - observed)).
    /// For Precipitation / WindSpeed: returns the oracle reading directly.
    ///
    /// Returns the index value scaled by 1_000_000.
    pub fn compute_from_reading(&self, oracle_reading_scaled: i64) -> i64 {
        match self {
            WeatherIndex::Temperature {
                kind,
                base_temp_scaled,
                ..
            } => match kind {
                TemperatureIndexKind::HDD => {
                    // HDD = max(0, base - observed)
                    let diff = base_temp_scaled - oracle_reading_scaled;
                    diff.max(0)
                }
                TemperatureIndexKind::CDD => {
                    // CDD = max(0, observed - base)
                    let diff = oracle_reading_scaled - base_temp_scaled;
                    diff.max(0)
                }
            },
            WeatherIndex::Precipitation { .. } | WeatherIndex::WindSpeed { .. } => {
                oracle_reading_scaled.max(0)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window() -> ObservationWindow {
        ObservationWindow::new(100, 200)
    }

    fn region() -> GeographicRegion {
        GeographicRegion::new("US-NYC")
    }

    /// Helper: scale a temperature in °F to internal representation (×1_000_000).
    fn temp(f: i64) -> i64 {
        f * 1_000_000
    }

    #[test]
    fn test_hdd_above_base() {
        // base = 65°F, observed = 50°F → HDD = 15
        let idx = WeatherIndex::Temperature {
            kind: TemperatureIndexKind::HDD,
            base_temp_scaled: temp(65),
            region: region(),
            window: window(),
        };
        assert_eq!(idx.compute_from_reading(temp(50)), temp(15));
    }

    #[test]
    fn test_hdd_below_base_gives_zero() {
        // base = 65°F, observed = 72°F → HDD = 0 (it's warm, no heating needed)
        let idx = WeatherIndex::Temperature {
            kind: TemperatureIndexKind::HDD,
            base_temp_scaled: temp(65),
            region: region(),
            window: window(),
        };
        assert_eq!(idx.compute_from_reading(temp(72)), 0);
    }

    #[test]
    fn test_cdd_above_base() {
        // base = 65°F, observed = 72°F → CDD = 7
        let idx = WeatherIndex::Temperature {
            kind: TemperatureIndexKind::CDD,
            base_temp_scaled: temp(65),
            region: region(),
            window: window(),
        };
        assert_eq!(idx.compute_from_reading(temp(72)), temp(7));
    }

    #[test]
    fn test_cdd_below_base_gives_zero() {
        // base = 65°F, observed = 50°F → CDD = 0
        let idx = WeatherIndex::Temperature {
            kind: TemperatureIndexKind::CDD,
            base_temp_scaled: temp(65),
            region: region(),
            window: window(),
        };
        assert_eq!(idx.compute_from_reading(temp(50)), 0);
    }

    #[test]
    fn test_precipitation_passthrough() {
        let idx = WeatherIndex::Precipitation {
            region: region(),
            window: window(),
        };
        // 120mm × 1_000_000
        assert_eq!(idx.compute_from_reading(120_000_000), 120_000_000);
    }

    #[test]
    fn test_wind_speed_passthrough() {
        let idx = WeatherIndex::WindSpeed {
            region: region(),
            window: window(),
        };
        assert_eq!(idx.compute_from_reading(5_500_000), 5_500_000);
    }

    #[test]
    fn test_observation_window_contains() {
        let w = ObservationWindow::new(100, 200);
        assert!(w.contains(100));
        assert!(w.contains(150));
        assert!(w.contains(200));
        assert!(!w.contains(99));
        assert!(!w.contains(201));
    }

    #[test]
    fn test_observation_window_is_closed() {
        let w = ObservationWindow::new(100, 200);
        assert!(!w.is_closed_at(200));
        assert!(w.is_closed_at(201));
    }
}
