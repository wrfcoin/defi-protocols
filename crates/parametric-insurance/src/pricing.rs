//! Actuarial premium pricing model.
//!
//! Premium = (Expected Loss) × (1 + Loading Factor)
//! Expected Loss = historical_trigger_frequency × payout
//!
//! All amounts are in WRF base units (u128).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use wrfcoin_defi_common::types::Amount;

use crate::error::{InsuranceError, InsuranceResult};
use crate::policy::WeatherTrigger;

/// Key identifying a trigger type for historical frequency lookup.
pub type TriggerTypeKey = String;

/// A historical frequency record for a given trigger type and region.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoricalRecord {
    /// Trigger type identifier (e.g., "TemperatureAbove").
    pub trigger_type: TriggerTypeKey,
    /// Geographic region key (from `GeoCoordinate::region_key()`).
    pub region: String,
    /// Number of times this trigger fired in historical data.
    pub trigger_count: u64,
    /// Total number of observation periods in historical data.
    pub total_periods: u64,
}

impl HistoricalRecord {
    pub fn new(
        trigger_type: impl Into<String>,
        region: impl Into<String>,
        trigger_count: u64,
        total_periods: u64,
    ) -> Self {
        Self {
            trigger_type: trigger_type.into(),
            region: region.into(),
            trigger_count,
            total_periods,
        }
    }

    /// Frequency as a fraction (trigger_count / total_periods).
    /// Returns 0 if total_periods is zero (no data — use default).
    pub fn frequency(&self) -> f64 {
        if self.total_periods == 0 {
            return 0.0;
        }
        self.trigger_count as f64 / self.total_periods as f64
    }
}

/// Actuarial premium pricing engine.
///
/// Computes premiums from historical trigger frequency + configurable loading factor.
/// The loading factor compensates for model uncertainty, profit margin, and expenses.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PremiumPricing {
    /// Historical frequency records, keyed by (trigger_type, region).
    records: HashMap<(TriggerTypeKey, String), HistoricalRecord>,
    /// Loading factor in basis points added on top of expected loss.
    /// E.g., 2000 bps = 20% loading → total multiplier = 1.20.
    loading_bps: u64,
    /// Default frequency (as bps of 10_000) used when no historical data is available.
    /// E.g., 500 bps = 5% default frequency.
    default_frequency_bps: u64,
}

impl PremiumPricing {
    /// Construct a pricing engine.
    ///
    /// - `loading_bps`: percentage above expected loss (e.g., 2000 = 20%)
    /// - `default_frequency_bps`: fallback frequency when no history exists
    pub fn new(loading_bps: u64, default_frequency_bps: u64) -> Self {
        Self {
            records: HashMap::new(),
            loading_bps,
            default_frequency_bps,
        }
    }

    /// Add a historical frequency record.
    pub fn add_record(&mut self, record: HistoricalRecord) {
        let key = (record.trigger_type.clone(), record.region.clone());
        self.records.insert(key, record);
    }

    /// Calculate the premium for a given payout amount, trigger type and region.
    ///
    /// Formula:
    ///   expected_loss = payout × frequency
    ///   premium = expected_loss × (10_000 + loading_bps) / 10_000
    ///
    /// Returns at least 1 base unit to avoid zero premiums.
    pub fn calculate_premium(
        &self,
        payout: Amount,
        trigger: &WeatherTrigger,
        region: &str,
    ) -> InsuranceResult<Amount> {
        if payout.is_zero() {
            return Err(InsuranceError::InvalidPayout {
                reason: "payout must be non-zero for pricing".into(),
            });
        }

        let trigger_type = trigger.type_key().to_string();
        let frequency = self
            .records
            .get(&(trigger_type, region.to_string()))
            .map(|r| r.frequency())
            .unwrap_or_else(|| self.default_frequency_bps as f64 / 10_000.0);

        // expected_loss = payout × frequency (scaled through integer arithmetic)
        // We use a scaled integer approach: freq_bps = round(frequency × 10_000)
        let freq_bps = (frequency * 10_000.0).round() as u128;

        // expected_loss_raw = payout × freq_bps / 10_000
        let expected_loss = payout
            .0
            .checked_mul(freq_bps)
            .and_then(|v| v.checked_div(10_000))
            .ok_or_else(|| InsuranceError::Overflow {
                operation: "expected_loss".into(),
            })?;

        // premium = expected_loss × (10_000 + loading_bps) / 10_000
        let total_bps = 10_000u128 + self.loading_bps as u128;
        let premium_raw = expected_loss
            .checked_mul(total_bps)
            .and_then(|v| v.checked_div(10_000))
            .ok_or_else(|| InsuranceError::Overflow {
                operation: "premium_loading".into(),
            })?;

        // Enforce minimum of 1 base unit.
        Ok(Amount::new(premium_raw.max(1)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::WeatherTrigger;

    #[test]
    fn premium_calculation_with_history() {
        let mut pricing = PremiumPricing::new(2000, 500); // 20% loading, 5% default
                                                          // 20 triggers in 100 periods → 20% frequency
        pricing.add_record(HistoricalRecord::new("TemperatureAbove", "40,-70", 20, 100));

        let payout = Amount::new(10_000_000); // 0.1 WRF
        let trigger = WeatherTrigger::TemperatureAbove(35.0);
        let premium = pricing
            .calculate_premium(payout, &trigger, "40,-70")
            .unwrap();

        // expected_loss = 10_000_000 × 2000/10_000 = 2_000_000
        // premium = 2_000_000 × 12_000/10_000 = 2_400_000
        assert_eq!(premium, Amount::new(2_400_000));
    }

    #[test]
    fn premium_calculation_default_frequency() {
        let pricing = PremiumPricing::new(1000, 500); // 10% loading, 5% default frequency
        let payout = Amount::new(100_000_000); // 1 WRF
        let trigger = WeatherTrigger::WindSpeedAbove(25.0);
        let premium = pricing
            .calculate_premium(payout, &trigger, "no-data-region")
            .unwrap();

        // expected_loss = 100_000_000 × 500/10_000 = 5_000_000
        // premium = 5_000_000 × 11_000/10_000 = 5_500_000
        assert_eq!(premium, Amount::new(5_500_000));
    }

    #[test]
    fn premium_zero_payout_fails() {
        let pricing = PremiumPricing::new(1000, 500);
        let trigger = WeatherTrigger::TemperatureAbove(35.0);
        let result = pricing.calculate_premium(Amount::ZERO, &trigger, "region");
        assert!(result.is_err());
    }

    #[test]
    fn premium_minimum_one_base_unit() {
        // Very low payout + zero historical frequency → premium should be at least 1
        let pricing = PremiumPricing::new(1000, 0); // 0 default frequency
        let payout = Amount::new(1);
        let trigger = WeatherTrigger::TemperatureAbove(35.0);
        let premium = pricing
            .calculate_premium(payout, &trigger, "no-data-region")
            .unwrap();
        assert_eq!(premium, Amount::new(1));
    }

    #[test]
    fn historical_record_frequency() {
        let r = HistoricalRecord::new("TemperatureAbove", "region", 25, 100);
        assert!((r.frequency() - 0.25).abs() < 1e-10);

        let r_zero = HistoricalRecord::new("TemperatureAbove", "region", 0, 0);
        assert_eq!(r_zero.frequency(), 0.0);
    }
}
