//! Oracle types: `WeatherReading` is a local stub type that mirrors core4's
//! weather observation structs conceptually. No cross-repo import is needed;
//! the settlement engine accepts any value that carries the relevant measurements.

use serde::{Deserialize, Serialize};

/// Geographic coordinates for a weather observation location.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct GeoCoordinate {
    /// Latitude in decimal degrees (-90.0 to 90.0).
    pub latitude: f64,
    /// Longitude in decimal degrees (-180.0 to 180.0).
    pub longitude: f64,
}

impl GeoCoordinate {
    pub fn new(latitude: f64, longitude: f64) -> Self {
        Self {
            latitude,
            longitude,
        }
    }

    /// Returns a simple region key string for geographic diversification tracking.
    /// Bucketed to 10-degree grid cells.
    pub fn region_key(&self) -> String {
        let lat_bucket = (self.latitude / 10.0).floor() as i32 * 10;
        let lon_bucket = (self.longitude / 10.0).floor() as i32 * 10;
        format!("{lat_bucket},{lon_bucket}")
    }
}

/// An oracle-attested weather reading at a specific location and time.
///
/// This is a local stub that mirrors core4's `WeatherObservation` semantics.
/// All fields use `Option<f64>` so that sensors missing a measurement do not
/// prevent settlement of triggers that don't rely on that measurement.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WeatherReading {
    /// Unix timestamp (seconds) of the observation.
    pub timestamp: u64,
    /// Geographic location of the measurement.
    pub location: GeoCoordinate,
    /// Temperature in degrees Celsius.
    pub temperature_celsius: Option<f64>,
    /// Precipitation in millimeters over the observation window.
    pub precipitation_mm: Option<f64>,
    /// Wind speed in meters per second.
    pub wind_speed_mps: Option<f64>,
    /// Standardised Precipitation-Evapotranspiration Index (SPEI); negative
    /// values indicate drought conditions.
    pub drought_index: Option<f64>,
    /// Whether this reading has been attested by a quorum of oracle nodes.
    pub oracle_attested: bool,
}

impl WeatherReading {
    /// Create a fully-attested reading with all measurements.
    pub fn new_attested(
        timestamp: u64,
        location: GeoCoordinate,
        temperature_celsius: Option<f64>,
        precipitation_mm: Option<f64>,
        wind_speed_mps: Option<f64>,
        drought_index: Option<f64>,
    ) -> Self {
        Self {
            timestamp,
            location,
            temperature_celsius,
            precipitation_mm,
            wind_speed_mps,
            drought_index,
            oracle_attested: true,
        }
    }
}
