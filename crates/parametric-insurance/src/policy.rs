//! Policy definitions: `WeatherTrigger`, `ParametricPolicy`, `PolicyStore`.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use wrfcoin_defi_common::types::{Address, Amount, AssetId, Timestamp};

use crate::error::{InsuranceError, InsuranceResult};
use crate::oracle::{GeoCoordinate, WeatherReading};

/// Unique identifier for a parametric insurance policy.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PolicyId(pub String);

impl PolicyId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for PolicyId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "policy#{}", self.0)
    }
}

/// Logic combinator for `CompositeTrigger`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CompositeLogic {
    /// All sub-triggers must be satisfied (logical AND).
    All,
    /// At least one sub-trigger must be satisfied (logical OR).
    Any,
}

/// Conditions that trigger an insurance payout.
///
/// Mirrors the `WeatherTrigger` enum in `ParametricInsurance.sol` (#98).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum WeatherTrigger {
    /// Payout if the observed temperature exceeds this threshold (°C).
    TemperatureAbove(f64),
    /// Payout if the observed temperature falls below this threshold (°C).
    TemperatureBelow(f64),
    /// Payout if observed precipitation exceeds this threshold (mm).
    PrecipitationAbove(f64),
    /// Payout if observed wind speed exceeds this threshold (m/s).
    WindSpeedAbove(f64),
    /// Payout if drought index falls below this threshold (SPEI; negative = drought).
    DroughtIndex(f64),
    /// Payout based on a set of sub-triggers combined with `All` or `Any` logic.
    CompositeTrigger {
        triggers: Vec<WeatherTrigger>,
        logic: CompositeLogic,
    },
}

impl WeatherTrigger {
    /// Evaluate this trigger against an attested oracle reading.
    ///
    /// Returns `Ok(true)` if the trigger fires, `Ok(false)` if not,
    /// and `Err` if the reading is missing the required measurement.
    pub fn evaluate(&self, reading: &WeatherReading) -> InsuranceResult<bool> {
        match self {
            WeatherTrigger::TemperatureAbove(threshold) => {
                let temp = reading.temperature_celsius.ok_or(InsuranceError::MissingOracleReading)?;
                Ok(temp > *threshold)
            }
            WeatherTrigger::TemperatureBelow(threshold) => {
                let temp = reading.temperature_celsius.ok_or(InsuranceError::MissingOracleReading)?;
                Ok(temp < *threshold)
            }
            WeatherTrigger::PrecipitationAbove(threshold) => {
                let precip = reading.precipitation_mm.ok_or(InsuranceError::MissingOracleReading)?;
                Ok(precip > *threshold)
            }
            WeatherTrigger::WindSpeedAbove(threshold) => {
                let wind = reading.wind_speed_mps.ok_or(InsuranceError::MissingOracleReading)?;
                Ok(wind > *threshold)
            }
            WeatherTrigger::DroughtIndex(threshold) => {
                let drought = reading.drought_index.ok_or(InsuranceError::MissingOracleReading)?;
                Ok(drought < *threshold)
            }
            WeatherTrigger::CompositeTrigger { triggers, logic } => {
                if triggers.is_empty() {
                    return Err(InsuranceError::InvalidTrigger {
                        reason: "CompositeTrigger must have at least one sub-trigger".into(),
                    });
                }
                match logic {
                    CompositeLogic::All => {
                        for t in triggers {
                            if !t.evaluate(reading)? {
                                return Ok(false);
                            }
                        }
                        Ok(true)
                    }
                    CompositeLogic::Any => {
                        for t in triggers {
                            if t.evaluate(reading)? {
                                return Ok(true);
                            }
                        }
                        Ok(false)
                    }
                }
            }
        }
    }

    /// Return a stable key representing the trigger variant (ignoring threshold).
    /// Used by risk limits to track exposure per trigger type.
    pub fn type_key(&self) -> &'static str {
        match self {
            WeatherTrigger::TemperatureAbove(_) => "TemperatureAbove",
            WeatherTrigger::TemperatureBelow(_) => "TemperatureBelow",
            WeatherTrigger::PrecipitationAbove(_) => "PrecipitationAbove",
            WeatherTrigger::WindSpeedAbove(_) => "WindSpeedAbove",
            WeatherTrigger::DroughtIndex(_) => "DroughtIndex",
            WeatherTrigger::CompositeTrigger { .. } => "CompositeTrigger",
        }
    }
}

/// Observation window: the period over which weather is monitored.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObservationWindow {
    pub start: Timestamp,
    pub end: Timestamp,
}

impl ObservationWindow {
    pub fn new(start: Timestamp, end: Timestamp) -> Self {
        Self { start, end }
    }

    /// True if the window has fully elapsed (end <= now).
    pub fn is_closed(&self, now: Timestamp) -> bool {
        now >= self.end
    }
}

/// Lifecycle state of a policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PolicyState {
    Active,
    TriggeredPaid,
    Expired,
}

impl std::fmt::Display for PolicyState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PolicyState::Active => write!(f, "Active"),
            PolicyState::TriggeredPaid => write!(f, "TriggeredPaid"),
            PolicyState::Expired => write!(f, "Expired"),
        }
    }
}

/// A parametric insurance policy purchased by a buyer.
///
/// Mirrors `ParametricInsurance.sol` `Policy` struct (smart-contracts #98).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParametricPolicy {
    /// Unique identifier.
    pub policy_id: PolicyId,
    /// Address of the policy buyer.
    pub buyer: Address,
    /// Premium paid at policy creation (WRF base units).
    pub premium: Amount,
    /// Maximum payout if trigger fires (WRF base units).
    pub payout: Amount,
    /// The weather condition that triggers payout.
    pub trigger: WeatherTrigger,
    /// The observation window.
    pub observation_window: ObservationWindow,
    /// Asset used for settlement (typically AssetId("WRF")).
    pub settlement_asset: AssetId,
    /// Location covered by this policy.
    pub location: GeoCoordinate,
    /// Current policy state.
    pub state: PolicyState,
}

impl ParametricPolicy {
    /// Create a new active policy.
    pub fn new(
        policy_id: PolicyId,
        buyer: Address,
        premium: Amount,
        payout: Amount,
        trigger: WeatherTrigger,
        observation_window: ObservationWindow,
        settlement_asset: AssetId,
        location: GeoCoordinate,
    ) -> InsuranceResult<Self> {
        if premium.is_zero() {
            return Err(InsuranceError::InvalidPremium {
                reason: "premium must be non-zero".into(),
            });
        }
        if payout.is_zero() {
            return Err(InsuranceError::InvalidPayout {
                reason: "payout must be non-zero".into(),
            });
        }
        Ok(Self {
            policy_id,
            buyer,
            premium,
            payout,
            trigger,
            observation_window,
            settlement_asset,
            location,
            state: PolicyState::Active,
        })
    }
}

/// Registry of all parametric policies, keyed by `PolicyId`.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct PolicyStore {
    policies: HashMap<PolicyId, ParametricPolicy>,
}

impl PolicyStore {
    pub fn new() -> Self {
        Self {
            policies: HashMap::new(),
        }
    }

    /// Register a new policy. Fails if a policy with the same ID already exists.
    pub fn register(&mut self, policy: ParametricPolicy) -> InsuranceResult<()> {
        if self.policies.contains_key(&policy.policy_id) {
            return Err(InsuranceError::PolicyNotFound {
                policy_id: policy.policy_id.as_str().to_string(),
            });
        }
        self.policies.insert(policy.policy_id.clone(), policy);
        Ok(())
    }

    /// Look up a policy by ID.
    pub fn get(&self, id: &PolicyId) -> InsuranceResult<&ParametricPolicy> {
        self.policies.get(id).ok_or_else(|| InsuranceError::PolicyNotFound {
            policy_id: id.as_str().to_string(),
        })
    }

    /// Look up a policy mutably.
    pub fn get_mut(&mut self, id: &PolicyId) -> InsuranceResult<&mut ParametricPolicy> {
        self.policies.get_mut(id).ok_or_else(|| InsuranceError::PolicyNotFound {
            policy_id: id.as_str().to_string(),
        })
    }

    /// Iterate over all active policies.
    pub fn active_policies(&self) -> impl Iterator<Item = &ParametricPolicy> {
        self.policies.values().filter(|p| p.state == PolicyState::Active)
    }

    /// Total count of all policies (active + settled).
    pub fn len(&self) -> usize {
        self.policies.len()
    }

    pub fn is_empty(&self) -> bool {
        self.policies.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn coord() -> GeoCoordinate {
        GeoCoordinate::new(40.7, -74.0)
    }

    fn window(start: u64, end: u64) -> ObservationWindow {
        ObservationWindow::new(Timestamp::new(start), Timestamp::new(end))
    }

    fn reading_full(temp: f64, precip: f64, wind: f64, drought: f64) -> WeatherReading {
        use crate::oracle::WeatherReading;
        WeatherReading::new_attested(
            1000,
            coord(),
            Some(temp),
            Some(precip),
            Some(wind),
            Some(drought),
        )
    }

    #[test]
    fn trigger_temperature_above_fires() {
        let t = WeatherTrigger::TemperatureAbove(35.0);
        let r = reading_full(36.0, 0.0, 0.0, 0.0);
        assert!(t.evaluate(&r).unwrap());
    }

    #[test]
    fn trigger_temperature_above_no_fire() {
        let t = WeatherTrigger::TemperatureAbove(35.0);
        let r = reading_full(34.9, 0.0, 0.0, 0.0);
        assert!(!t.evaluate(&r).unwrap());
    }

    #[test]
    fn trigger_temperature_below_fires() {
        let t = WeatherTrigger::TemperatureBelow(-10.0);
        let r = reading_full(-15.0, 0.0, 0.0, 0.0);
        assert!(t.evaluate(&r).unwrap());
    }

    #[test]
    fn trigger_temperature_below_no_fire() {
        let t = WeatherTrigger::TemperatureBelow(-10.0);
        let r = reading_full(-5.0, 0.0, 0.0, 0.0);
        assert!(!t.evaluate(&r).unwrap());
    }

    #[test]
    fn trigger_precipitation_above_fires() {
        let t = WeatherTrigger::PrecipitationAbove(100.0);
        let r = reading_full(20.0, 150.0, 0.0, 0.0);
        assert!(t.evaluate(&r).unwrap());
    }

    #[test]
    fn trigger_precipitation_above_no_fire() {
        let t = WeatherTrigger::PrecipitationAbove(100.0);
        let r = reading_full(20.0, 50.0, 0.0, 0.0);
        assert!(!t.evaluate(&r).unwrap());
    }

    #[test]
    fn trigger_wind_speed_above_fires() {
        let t = WeatherTrigger::WindSpeedAbove(25.0);
        let r = reading_full(20.0, 0.0, 30.0, 0.0);
        assert!(t.evaluate(&r).unwrap());
    }

    #[test]
    fn trigger_wind_speed_above_no_fire() {
        let t = WeatherTrigger::WindSpeedAbove(25.0);
        let r = reading_full(20.0, 0.0, 20.0, 0.0);
        assert!(!t.evaluate(&r).unwrap());
    }

    #[test]
    fn trigger_drought_index_fires() {
        // SPEI < -1.5 => drought
        let t = WeatherTrigger::DroughtIndex(-1.5);
        let r = reading_full(25.0, 5.0, 3.0, -2.0);
        assert!(t.evaluate(&r).unwrap());
    }

    #[test]
    fn trigger_drought_index_no_fire() {
        let t = WeatherTrigger::DroughtIndex(-1.5);
        let r = reading_full(25.0, 5.0, 3.0, -1.0);
        assert!(!t.evaluate(&r).unwrap());
    }

    #[test]
    fn trigger_composite_all_fires_when_all_met() {
        let t = WeatherTrigger::CompositeTrigger {
            triggers: vec![
                WeatherTrigger::TemperatureAbove(30.0),
                WeatherTrigger::PrecipitationAbove(50.0),
            ],
            logic: CompositeLogic::All,
        };
        let r = reading_full(35.0, 80.0, 0.0, 0.0);
        assert!(t.evaluate(&r).unwrap());
    }

    #[test]
    fn trigger_composite_all_no_fire_when_one_unmet() {
        let t = WeatherTrigger::CompositeTrigger {
            triggers: vec![
                WeatherTrigger::TemperatureAbove(30.0),
                WeatherTrigger::PrecipitationAbove(50.0),
            ],
            logic: CompositeLogic::All,
        };
        let r = reading_full(35.0, 30.0, 0.0, 0.0); // precip not met
        assert!(!t.evaluate(&r).unwrap());
    }

    #[test]
    fn trigger_composite_any_fires_when_one_met() {
        let t = WeatherTrigger::CompositeTrigger {
            triggers: vec![
                WeatherTrigger::TemperatureAbove(40.0),
                WeatherTrigger::WindSpeedAbove(25.0),
            ],
            logic: CompositeLogic::Any,
        };
        let r = reading_full(35.0, 0.0, 30.0, 0.0); // only wind met
        assert!(t.evaluate(&r).unwrap());
    }

    #[test]
    fn trigger_composite_any_no_fire_when_none_met() {
        let t = WeatherTrigger::CompositeTrigger {
            triggers: vec![
                WeatherTrigger::TemperatureAbove(40.0),
                WeatherTrigger::WindSpeedAbove(25.0),
            ],
            logic: CompositeLogic::Any,
        };
        let r = reading_full(35.0, 0.0, 10.0, 0.0);
        assert!(!t.evaluate(&r).unwrap());
    }

    #[test]
    fn trigger_missing_reading_returns_error() {
        use crate::oracle::WeatherReading;
        let t = WeatherTrigger::TemperatureAbove(30.0);
        let r = WeatherReading {
            timestamp: 1000,
            location: coord(),
            temperature_celsius: None,
            precipitation_mm: None,
            wind_speed_mps: None,
            drought_index: None,
            oracle_attested: true,
        };
        assert!(t.evaluate(&r).is_err());
    }

    #[test]
    fn composite_empty_triggers_returns_error() {
        let t = WeatherTrigger::CompositeTrigger {
            triggers: vec![],
            logic: CompositeLogic::All,
        };
        let r = reading_full(20.0, 0.0, 0.0, 0.0);
        assert!(t.evaluate(&r).is_err());
    }

    #[test]
    fn observation_window_closed() {
        let w = window(100, 200);
        assert!(!w.is_closed(Timestamp::new(150)));
        assert!(w.is_closed(Timestamp::new(200)));
        assert!(w.is_closed(Timestamp::new(300)));
    }

    #[test]
    fn policy_store_register_and_get() {
        let mut store = PolicyStore::new();
        let policy = ParametricPolicy::new(
            PolicyId::new("p1"),
            Address::zero(),
            Amount::new(1_000_000),
            Amount::new(10_000_000),
            WeatherTrigger::TemperatureAbove(35.0),
            window(100, 200),
            AssetId::new("WRF"),
            coord(),
        )
        .unwrap();
        store.register(policy).unwrap();
        let p = store.get(&PolicyId::new("p1")).unwrap();
        assert_eq!(p.state, PolicyState::Active);
    }

    #[test]
    fn policy_rejects_zero_premium() {
        let result = ParametricPolicy::new(
            PolicyId::new("p2"),
            Address::zero(),
            Amount::ZERO,
            Amount::new(10_000_000),
            WeatherTrigger::TemperatureAbove(35.0),
            window(100, 200),
            AssetId::new("WRF"),
            coord(),
        );
        assert!(result.is_err());
    }

    #[test]
    fn policy_rejects_zero_payout() {
        let result = ParametricPolicy::new(
            PolicyId::new("p3"),
            Address::zero(),
            Amount::new(1_000_000),
            Amount::ZERO,
            WeatherTrigger::TemperatureAbove(35.0),
            window(100, 200),
            AssetId::new("WRF"),
            coord(),
        );
        assert!(result.is_err());
    }

    #[test]
    fn geo_coordinate_region_key() {
        let c = GeoCoordinate::new(45.3, -73.8);
        assert_eq!(c.region_key(), "40,-80");
        let c2 = GeoCoordinate::new(-5.0, 10.0);
        assert_eq!(c2.region_key(), "-10,10");
    }
}
