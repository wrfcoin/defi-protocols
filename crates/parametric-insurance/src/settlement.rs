//! `SettlementEngine`: evaluates weather triggers against oracle readings and
//! executes payouts.
//!
//! On settlement:
//! 1. Verify the observation window has closed.
//! 2. Verify the reading is oracle-attested.
//! 3. Evaluate the policy's `WeatherTrigger`.
//! 4. If triggered → execute payout from vault, mark policy `TriggeredPaid`.
//! 5. If not triggered → release reserved exposure, mark policy `Expired`.

use serde::{Deserialize, Serialize};
use wrfcoin_defi_common::types::Timestamp;

use crate::error::{InsuranceError, InsuranceResult};
use crate::oracle::WeatherReading;
use crate::policy::{PolicyId, PolicyState, PolicyStore};
use crate::vault::InsuranceVault;

/// Result of a settlement attempt.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SettlementOutcome {
    /// Trigger fired — payout executed.
    Triggered {
        policy_id: PolicyId,
        payout_amount: u128,
    },
    /// Trigger did not fire — policy expired without payout.
    Expired { policy_id: PolicyId },
}

/// Engine that processes oracle readings and settles policies.
pub struct SettlementEngine;

impl SettlementEngine {
    /// Attempt to settle a single policy.
    ///
    /// - `now`: current timestamp (must be ≥ observation window end).
    /// - `reading`: oracle-attested weather reading for the covered location/period.
    pub fn settle(
        policy_store: &mut PolicyStore,
        vault: &mut InsuranceVault,
        policy_id: &PolicyId,
        reading: &WeatherReading,
        now: Timestamp,
    ) -> InsuranceResult<SettlementOutcome> {
        // Verify the policy exists and is active.
        let policy = policy_store.get(policy_id)?;
        if policy.state != PolicyState::Active {
            return Err(InsuranceError::PolicyNotActive {
                policy_id: policy_id.as_str().to_string(),
                state: policy.state.to_string(),
            });
        }

        // Verify window has closed.
        if !policy.observation_window.is_closed(now) {
            return Err(InsuranceError::WindowNotClosed {
                policy_id: policy_id.as_str().to_string(),
            });
        }

        // Verify oracle attestation.
        if !reading.oracle_attested {
            return Err(InsuranceError::MissingOracleReading);
        }

        // Clone fields needed after mutable borrow.
        let trigger_type_key = policy.trigger.type_key().to_string();
        let region_key = policy.location.region_key();
        let payout = policy.payout;
        let trigger = policy.trigger.clone();

        // Evaluate trigger.
        let triggered = trigger.evaluate(reading)?;

        // Mutate policy state.
        {
            let policy_mut = policy_store.get_mut(policy_id)?;
            if triggered {
                policy_mut.state = PolicyState::TriggeredPaid;
            } else {
                policy_mut.state = PolicyState::Expired;
            }
        }

        if triggered {
            // Release reserved exposure then execute payout.
            vault.release_exposure(payout, &trigger_type_key, &region_key);
            vault.execute_payout(payout)?;
            Ok(SettlementOutcome::Triggered {
                policy_id: policy_id.clone(),
                payout_amount: payout.0,
            })
        } else {
            // Just release exposure — no payout.
            vault.release_exposure(payout, &trigger_type_key, &region_key);
            Ok(SettlementOutcome::Expired {
                policy_id: policy_id.clone(),
            })
        }
    }

    /// Settle all active policies whose observation windows have closed.
    ///
    /// The caller provides a closure that, given a `PolicyId`, returns the relevant
    /// oracle reading. If the closure returns `None` the policy is skipped.
    pub fn settle_batch<F>(
        policy_store: &mut PolicyStore,
        vault: &mut InsuranceVault,
        now: Timestamp,
        reading_for: &mut F,
    ) -> Vec<InsuranceResult<SettlementOutcome>>
    where
        F: FnMut(&PolicyId) -> Option<WeatherReading>,
    {
        // Collect IDs of policies eligible for settlement (window closed + active).
        let eligible: Vec<PolicyId> = policy_store
            .active_policies()
            .filter(|p| p.observation_window.is_closed(now))
            .map(|p| p.policy_id.clone())
            .collect();

        eligible
            .into_iter()
            .map(|id| {
                if let Some(reading) = reading_for(&id) {
                    Self::settle(policy_store, vault, &id, &reading, now)
                } else {
                    Err(InsuranceError::MissingOracleReading)
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oracle::{GeoCoordinate, WeatherReading};
    use crate::policy::{
        CompositeLogic, ObservationWindow, ParametricPolicy, PolicyStore, WeatherTrigger,
    };
    use crate::vault::{InsuranceVault, RiskLimits, VaultConfig};
    use wrfcoin_defi_common::types::{Address, Amount, AssetId, Timestamp};

    fn addr(seed: u8) -> Address {
        let mut bytes = [0u8; 32];
        bytes[31] = seed;
        Address::from_bytes(bytes)
    }

    fn coord() -> GeoCoordinate {
        GeoCoordinate::new(40.7, -74.0)
    }

    fn window(start: u64, end: u64) -> ObservationWindow {
        ObservationWindow::new(Timestamp::new(start), Timestamp::new(end))
    }

    fn reading_attested(
        temp: f64,
        precip: f64,
        wind: f64,
        drought: f64,
    ) -> WeatherReading {
        WeatherReading::new_attested(
            1000,
            coord(),
            Some(temp),
            Some(precip),
            Some(wind),
            Some(drought),
        )
    }

    /// Build a vault pre-loaded with enough capital.
    fn funded_vault(capital: u128) -> InsuranceVault {
        let mut v = InsuranceVault::new(
            VaultConfig { min_capital_ratio_bps: 5_000 }, // 50% minimum
            RiskLimits {
                max_exposure_per_trigger_type: Amount::new(u128::MAX / 2),
                max_exposure_per_region: Amount::new(u128::MAX / 2),
            },
        );
        v.deposit(addr(99), Amount::new(capital)).unwrap();
        v
    }

    fn make_policy(id: &str, payout: u128, trigger: WeatherTrigger) -> ParametricPolicy {
        ParametricPolicy::new(
            PolicyId::new(id),
            addr(1),
            Amount::new(1_000_000),
            Amount::new(payout),
            trigger,
            window(100, 200),
            AssetId::new("WRF"),
            coord(),
        )
        .unwrap()
    }

    // Helper: register policy and reserve exposure in vault.
    fn setup_policy(
        store: &mut PolicyStore,
        vault: &mut InsuranceVault,
        policy: ParametricPolicy,
    ) {
        let trigger_key = policy.trigger.type_key().to_string();
        let region_key = policy.location.region_key();
        let payout = policy.payout;
        store.register(policy).unwrap();
        vault.collect_premium(Amount::new(1_000_000)).unwrap();
        vault
            .reserve_exposure(payout, &trigger_key, &region_key)
            .unwrap();
    }

    // ── Happy path: trigger fires ──────────────────────────────────────────

    #[test]
    fn settlement_triggered_payout_executed() {
        let mut store = PolicyStore::new();
        let mut vault = funded_vault(100_000_000);

        let policy = make_policy("p1", 50_000_000, WeatherTrigger::TemperatureAbove(35.0));
        setup_policy(&mut store, &mut vault, policy);

        let before_assets = vault.total_assets;
        let reading = reading_attested(40.0, 0.0, 0.0, 0.0); // temp above 35
        let outcome = SettlementEngine::settle(
            &mut store,
            &mut vault,
            &PolicyId::new("p1"),
            &reading,
            Timestamp::new(200),
        )
        .unwrap();

        assert_eq!(
            outcome,
            SettlementOutcome::Triggered {
                policy_id: PolicyId::new("p1"),
                payout_amount: 50_000_000
            }
        );
        assert_eq!(vault.total_assets, Amount::new(before_assets.0 - 50_000_000));
        assert_eq!(store.get(&PolicyId::new("p1")).unwrap().state, PolicyState::TriggeredPaid);
    }

    // ── Miss path: trigger does not fire ──────────────────────────────────

    #[test]
    fn settlement_not_triggered_policy_expires() {
        let mut store = PolicyStore::new();
        let mut vault = funded_vault(100_000_000);

        let policy = make_policy("p2", 50_000_000, WeatherTrigger::TemperatureAbove(35.0));
        setup_policy(&mut store, &mut vault, policy);

        let before_assets = vault.total_assets;
        let reading = reading_attested(30.0, 0.0, 0.0, 0.0); // temp below 35
        let outcome = SettlementEngine::settle(
            &mut store,
            &mut vault,
            &PolicyId::new("p2"),
            &reading,
            Timestamp::new(200),
        )
        .unwrap();

        assert_eq!(
            outcome,
            SettlementOutcome::Expired { policy_id: PolicyId::new("p2") }
        );
        // Assets unchanged (no payout).
        assert_eq!(vault.total_assets, before_assets);
        assert_eq!(store.get(&PolicyId::new("p2")).unwrap().state, PolicyState::Expired);
    }

    // ── Window not closed ─────────────────────────────────────────────────

    #[test]
    fn settlement_fails_if_window_open() {
        let mut store = PolicyStore::new();
        let mut vault = funded_vault(100_000_000);

        let policy = make_policy("p3", 10_000_000, WeatherTrigger::TemperatureAbove(35.0));
        setup_policy(&mut store, &mut vault, policy);

        let reading = reading_attested(40.0, 0.0, 0.0, 0.0);
        let result = SettlementEngine::settle(
            &mut store,
            &mut vault,
            &PolicyId::new("p3"),
            &reading,
            Timestamp::new(150), // before window end (200)
        );
        assert!(result.is_err());
    }

    // ── Non-attested oracle reading ────────────────────────────────────────

    #[test]
    fn settlement_fails_on_unattested_reading() {
        let mut store = PolicyStore::new();
        let mut vault = funded_vault(100_000_000);

        let policy = make_policy("p4", 10_000_000, WeatherTrigger::TemperatureAbove(35.0));
        setup_policy(&mut store, &mut vault, policy);

        let mut reading = reading_attested(40.0, 0.0, 0.0, 0.0);
        reading.oracle_attested = false;

        let result = SettlementEngine::settle(
            &mut store,
            &mut vault,
            &PolicyId::new("p4"),
            &reading,
            Timestamp::new(200),
        );
        assert!(result.is_err());
    }

    // ── Already settled policy ─────────────────────────────────────────────

    #[test]
    fn settlement_fails_on_already_settled_policy() {
        let mut store = PolicyStore::new();
        let mut vault = funded_vault(100_000_000);

        let policy = make_policy("p5", 10_000_000, WeatherTrigger::TemperatureAbove(35.0));
        setup_policy(&mut store, &mut vault, policy);

        let reading = reading_attested(40.0, 0.0, 0.0, 0.0);
        // First settlement.
        SettlementEngine::settle(
            &mut store,
            &mut vault,
            &PolicyId::new("p5"),
            &reading,
            Timestamp::new(200),
        )
        .unwrap();

        // Second attempt should fail.
        let result = SettlementEngine::settle(
            &mut store,
            &mut vault,
            &PolicyId::new("p5"),
            &reading,
            Timestamp::new(200),
        );
        assert!(result.is_err());
    }

    // ── Composite trigger settlement ───────────────────────────────────────

    #[test]
    fn settlement_composite_all_triggers() {
        let mut store = PolicyStore::new();
        let mut vault = funded_vault(200_000_000);

        let trigger = WeatherTrigger::CompositeTrigger {
            triggers: vec![
                WeatherTrigger::TemperatureAbove(30.0),
                WeatherTrigger::WindSpeedAbove(20.0),
            ],
            logic: CompositeLogic::All,
        };
        let policy = make_policy("p6", 100_000_000, trigger);
        setup_policy(&mut store, &mut vault, policy);

        let reading = reading_attested(35.0, 0.0, 25.0, 0.0); // both met
        let outcome = SettlementEngine::settle(
            &mut store,
            &mut vault,
            &PolicyId::new("p6"),
            &reading,
            Timestamp::new(200),
        )
        .unwrap();
        assert!(matches!(outcome, SettlementOutcome::Triggered { .. }));
    }

    #[test]
    fn settlement_composite_any_partial_trigger() {
        let mut store = PolicyStore::new();
        let mut vault = funded_vault(200_000_000);

        let trigger = WeatherTrigger::CompositeTrigger {
            triggers: vec![
                WeatherTrigger::TemperatureAbove(40.0), // not met
                WeatherTrigger::WindSpeedAbove(20.0),   // met
            ],
            logic: CompositeLogic::Any,
        };
        let policy = make_policy("p7", 100_000_000, trigger);
        setup_policy(&mut store, &mut vault, policy);

        let reading = reading_attested(35.0, 0.0, 25.0, 0.0); // only wind met
        let outcome = SettlementEngine::settle(
            &mut store,
            &mut vault,
            &PolicyId::new("p7"),
            &reading,
            Timestamp::new(200),
        )
        .unwrap();
        assert!(matches!(outcome, SettlementOutcome::Triggered { .. }));
    }

    // ── Batch settlement ───────────────────────────────────────────────────

    #[test]
    fn batch_settlement_settles_closed_windows() {
        let mut store = PolicyStore::new();
        let mut vault = funded_vault(200_000_000);

        let p1 = make_policy("b1", 50_000_000, WeatherTrigger::TemperatureAbove(35.0));
        let p2 = make_policy("b2", 50_000_000, WeatherTrigger::WindSpeedAbove(20.0));
        setup_policy(&mut store, &mut vault, p1);
        setup_policy(&mut store, &mut vault, p2);

        let reading = reading_attested(40.0, 0.0, 25.0, 0.0);

        let outcomes = SettlementEngine::settle_batch(
            &mut store,
            &mut vault,
            Timestamp::new(200),
            &mut |_| Some(reading.clone()),
        );

        assert_eq!(outcomes.len(), 2);
        for outcome in &outcomes {
            assert!(outcome.is_ok());
        }
    }
}
