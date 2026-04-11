//! European-style weather options with Black-76 adapted pricing.
//!
//! ## Pricing approach
//! Black-76 (Black's 1976 model for options on futures) is used with f64 arithmetic.
//! This is acceptable because:
//! 1. Option _pricing_ is advisory — it determines the premium quoted, not settlement.
//! 2. Settlement itself is integer arithmetic (see `SettlementEngine`).
//! 3. No external math crates are used; the standard normal CDF is approximated with
//!    a 6-coefficient rational polynomial (Hart, Abramowitz & Stegun 26.2.17,
//!    max error ≈ 7.5e-8).
//!
//! ## Black-76 formula
//! For a European call/put on a futures price F with strike K:
//! - d1 = (ln(F/K) + 0.5 * σ² * T) / (σ * √T)
//! - d2 = d1 − σ * √T
//! - call = e^(-rT) * [F * Φ(d1) − K * Φ(d2)]
//! - put  = e^(-rT) * [K * Φ(−d2) − F * Φ(−d1)]
//!
//! For weather indices there is no dividend yield.  When r ≈ 0, e^(-rT) ≈ 1,
//! so we expose `risk_free_rate` as a parameter but it defaults to 0.0 in tests.

use std::collections::HashMap;
use serde::{Deserialize, Serialize};
use wrfcoin_defi_common::types::{Address, Amount};
use crate::error::{DerivativesError, DerivativesResult};
use crate::futures::FutureId;

/// Call or Put option type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OptionType {
    Call,
    Put,
}

/// Parameters for Black-76 pricing.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Black76Params {
    /// Current futures price (index units, un-scaled, i.e., actual degree-days etc.).
    pub futures_price: f64,
    /// Strike price in the same units as futures_price.
    pub strike: f64,
    /// Annualised volatility (e.g., 0.30 for 30%).
    pub volatility: f64,
    /// Time to expiry in years (e.g., 0.25 for 3 months).
    pub time_to_expiry_years: f64,
    /// Continuous risk-free rate (e.g., 0.05 for 5%). Use 0.0 for WRF context.
    pub risk_free_rate: f64,
}

/// Standard normal CDF approximation (Abramowitz & Stegun 26.2.17).
/// Max error ≈ 7.5e-8.
fn norm_cdf(x: f64) -> f64 {
    const P: f64 = 0.2316419;
    const A1: f64 = 0.319381530;
    const A2: f64 = -0.356563782;
    const A3: f64 = 1.781477937;
    const A4: f64 = -1.821255978;
    const A5: f64 = 1.330274429;

    let abs_x = x.abs();
    let t = 1.0 / (1.0 + P * abs_x);
    let poly = t * (A1 + t * (A2 + t * (A3 + t * (A4 + t * A5))));
    let pdf = (-0.5 * abs_x * abs_x).exp() / (2.0_f64 * std::f64::consts::PI).sqrt();
    let cdf_pos = 1.0 - pdf * poly;
    if x >= 0.0 {
        cdf_pos
    } else {
        1.0 - cdf_pos
    }
}

impl Black76Params {
    /// Compute the option price using Black-76.
    /// Returns `None` if parameters are degenerate (e.g., T ≤ 0, σ ≤ 0, F ≤ 0, K ≤ 0).
    pub fn price(&self, option_type: OptionType) -> Option<f64> {
        let f = self.futures_price;
        let k = self.strike;
        let sigma = self.volatility;
        let t = self.time_to_expiry_years;
        let r = self.risk_free_rate;

        if t <= 0.0 || sigma <= 0.0 || f <= 0.0 || k <= 0.0 {
            return None;
        }

        let sqrt_t = t.sqrt();
        let d1 = (f / k).ln() / (sigma * sqrt_t) + 0.5 * sigma * sqrt_t;
        let d2 = d1 - sigma * sqrt_t;
        let discount = (-r * t).exp();

        let price = match option_type {
            OptionType::Call => discount * (f * norm_cdf(d1) - k * norm_cdf(d2)),
            OptionType::Put => discount * (k * norm_cdf(-d2) - f * norm_cdf(-d1)),
        };

        // Clamp to non-negative (rounding / approximation artifacts near ATM boundary)
        Some(price.max(0.0))
    }
}

/// Unique identifier for an options contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct OptionId(pub u64);

impl std::fmt::Display for OptionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "option#{}", self.0)
    }
}

/// Status of a weather option.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OptionStatus {
    Active,
    Exercised,
    Expired,
}

/// A European-style weather option on a weather future.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WeatherOption {
    pub id: OptionId,
    /// The underlying futures contract.
    pub underlying_future_id: FutureId,
    /// Strike index value scaled by 1_000_000 (same units as oracle readings).
    pub strike_scaled: i64,
    /// Block height at which this option expires (must be <= settlement_block of future).
    pub expiry_block: u64,
    pub option_type: OptionType,
    /// Premium paid in WRF base units.
    pub premium: Amount,
    pub holder: Address,
    pub status: OptionStatus,
}

impl WeatherOption {
    pub fn is_active(&self) -> bool {
        matches!(self.status, OptionStatus::Active)
    }

    /// Compute intrinsic value at expiry (in index scaled units).
    /// Returns `max(0, final - strike)` for a call, `max(0, strike - final)` for a put.
    pub fn intrinsic_value_scaled(&self, final_index_scaled: i64) -> i64 {
        match self.option_type {
            OptionType::Call => (final_index_scaled - self.strike_scaled).max(0),
            OptionType::Put => (self.strike_scaled - final_index_scaled).max(0),
        }
    }
}

/// Engine managing open weather options and their exercise/expiry.
#[derive(Debug, Default)]
pub struct OptionsEngine {
    options: HashMap<OptionId, WeatherOption>,
    next_id: u64,
}

impl OptionsEngine {
    pub fn new() -> Self {
        Self::default()
    }

    /// Write (create) a new European weather option.
    pub fn write_option(
        &mut self,
        underlying_future_id: FutureId,
        strike_scaled: i64,
        expiry_block: u64,
        option_type: OptionType,
        premium: Amount,
        holder: Address,
    ) -> DerivativesResult<OptionId> {
        if strike_scaled <= 0 {
            return Err(DerivativesError::InvalidParameter {
                reason: "strike must be positive".into(),
            });
        }
        let id = OptionId(self.next_id);
        self.next_id += 1;
        self.options.insert(
            id,
            WeatherOption {
                id,
                underlying_future_id,
                strike_scaled,
                expiry_block,
                option_type,
                premium,
                holder,
                status: OptionStatus::Active,
            },
        );
        Ok(id)
    }

    pub fn get_option(&self, id: OptionId) -> Option<&WeatherOption> {
        self.options.get(&id)
    }

    /// Exercise (settle) a European option at expiry.
    /// Returns the payout in WRF base units (intrinsic value × contracts × tick_size).
    /// `tick_size` is the tick size of the underlying future.
    pub fn exercise(
        &mut self,
        option_id: OptionId,
        final_index_scaled: i64,
        current_block: u64,
        tick_size: Amount,
    ) -> DerivativesResult<Amount> {
        let opt = self
            .options
            .get_mut(&option_id)
            .ok_or(DerivativesError::OptionNotFound { id: option_id.0 })?;

        if !opt.is_active() {
            return Err(DerivativesError::AlreadySettled { id: option_id.0 });
        }
        if current_block < opt.expiry_block {
            return Err(DerivativesError::NotYetExpired {
                settlement_block: opt.expiry_block,
                current_block,
            });
        }

        let intrinsic = opt.intrinsic_value_scaled(final_index_scaled);
        if intrinsic <= 0 {
            opt.status = OptionStatus::Expired;
            return Ok(Amount::ZERO);
        }

        opt.status = OptionStatus::Exercised;
        // payout = intrinsic_scaled / 1_000_000 * tick_size
        let payout_raw = (intrinsic as u128)
            .checked_mul(tick_size.0)
            .and_then(|v| v.checked_div(1_000_000))
            .ok_or(DerivativesError::Overflow {
                operation: "option payout".into(),
            })?;
        Ok(Amount::new(payout_raw))
    }

    /// Price an option using Black-76.  Returns the premium in "index units"
    /// (not WRF base units) as an f64.  Callers multiply by tick_size to get WRF.
    pub fn price_black76(&self, params: Black76Params, option_type: OptionType) -> Option<f64> {
        params.price(option_type)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wrfcoin_defi_common::types::Address;

    fn addr(n: u8) -> Address {
        let mut b = [0u8; 32];
        b[31] = n;
        Address::from_bytes(b)
    }

    // ── Black-76 directional correctness ─────────────────────────────────────

    #[test]
    fn test_black76_call_positive_when_future_above_strike() {
        let params = Black76Params {
            futures_price: 80.0,
            strike: 65.0,
            volatility: 0.30,
            time_to_expiry_years: 0.25,
            risk_free_rate: 0.0,
        };
        let price = params.price(OptionType::Call).unwrap();
        assert!(price > 0.0, "ITM call should have positive price, got {price}");
    }

    #[test]
    fn test_black76_call_near_zero_when_future_below_strike() {
        // Deep OTM call
        let params = Black76Params {
            futures_price: 10.0,
            strike: 100.0,
            volatility: 0.30,
            time_to_expiry_years: 0.25,
            risk_free_rate: 0.0,
        };
        let price = params.price(OptionType::Call).unwrap();
        assert!(price < 1.0, "Deep OTM call should be near zero, got {price}");
    }

    #[test]
    fn test_black76_put_positive_when_future_below_strike() {
        let params = Black76Params {
            futures_price: 50.0,
            strike: 65.0,
            volatility: 0.30,
            time_to_expiry_years: 0.25,
            risk_free_rate: 0.0,
        };
        let price = params.price(OptionType::Put).unwrap();
        assert!(price > 0.0, "ITM put should have positive price, got {price}");
    }

    #[test]
    fn test_black76_put_near_zero_when_future_above_strike() {
        let params = Black76Params {
            futures_price: 150.0,
            strike: 65.0,
            volatility: 0.30,
            time_to_expiry_years: 0.25,
            risk_free_rate: 0.0,
        };
        let price = params.price(OptionType::Put).unwrap();
        assert!(price < 1.0, "Deep OTM put should be near zero, got {price}");
    }

    #[test]
    fn test_black76_degenerate_returns_none() {
        let base = Black76Params {
            futures_price: 80.0,
            strike: 65.0,
            volatility: 0.30,
            time_to_expiry_years: 0.0, // degenerate
            risk_free_rate: 0.0,
        };
        assert!(base.price(OptionType::Call).is_none());

        let base2 = Black76Params { volatility: 0.0, ..base };
        assert!(base2.price(OptionType::Call).is_none());
    }

    // ── Option exercise ───────────────────────────────────────────────────────

    #[test]
    fn test_call_exercise_in_the_money() {
        let mut engine = OptionsEngine::new();
        let fid = FutureId(0);
        // Strike = 65, final = 72 → intrinsic = 7 CDD → payout = 7 × tick_size
        let id = engine
            .write_option(fid, 65_000_000, 200, OptionType::Call, Amount::new(500), addr(1))
            .unwrap();

        // tick_size = 1_000_000 WRF base units per index unit
        let payout = engine
            .exercise(id, 72_000_000, 200, Amount::new(1_000_000))
            .unwrap();
        // 7 × 1_000_000 = 7_000_000
        assert_eq!(payout, Amount::new(7_000_000));
    }

    #[test]
    fn test_put_exercise_in_the_money() {
        let mut engine = OptionsEngine::new();
        let fid = FutureId(0);
        // Strike = 65, final = 50 → put intrinsic = 15 → payout = 15 × tick_size
        let id = engine
            .write_option(fid, 65_000_000, 200, OptionType::Put, Amount::new(500), addr(2))
            .unwrap();
        let payout = engine
            .exercise(id, 50_000_000, 200, Amount::new(1_000_000))
            .unwrap();
        assert_eq!(payout, Amount::new(15_000_000));
    }

    #[test]
    fn test_otm_option_expires_worthless() {
        let mut engine = OptionsEngine::new();
        let fid = FutureId(0);
        // Call strike = 65, final = 50 → OTM
        let id = engine
            .write_option(fid, 65_000_000, 200, OptionType::Call, Amount::new(100), addr(3))
            .unwrap();
        let payout = engine
            .exercise(id, 50_000_000, 200, Amount::new(1_000_000))
            .unwrap();
        assert_eq!(payout, Amount::ZERO);
        assert_eq!(engine.get_option(id).unwrap().status, OptionStatus::Expired);
    }

    #[test]
    fn test_exercise_before_expiry_fails() {
        let mut engine = OptionsEngine::new();
        let fid = FutureId(0);
        let id = engine
            .write_option(fid, 65_000_000, 200, OptionType::Call, Amount::new(100), addr(4))
            .unwrap();
        assert!(engine.exercise(id, 72_000_000, 199, Amount::new(1_000_000)).is_err());
    }
}
