use serde::{Deserialize, Serialize};
use std::fmt;
use std::num::TryFromIntError;
use std::ops::{Add, Sub};

/// Unique identifier for an asset within the DeFi system.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AssetId(pub String);

impl AssetId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for AssetId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Base units per WRF coin, matching core4's `WRF_UNITS_PER_COIN` (PR #505).
/// 1 WRF = 10^8 base units. All on-chain amounts are denominated in base units.
pub const WRF_UNITS_PER_COIN: u128 = 100_000_000;

/// Token amount represented as a u128 to support large values without floating point.
/// All amounts are in the smallest denomination (e.g., wei for ETH-like tokens).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Amount(pub u128);

impl Amount {
    pub const ZERO: Amount = Amount(0);

    pub fn new(value: u128) -> Self {
        Self(value)
    }

    pub fn is_zero(&self) -> bool {
        self.0 == 0
    }

    pub fn checked_add(self, rhs: Amount) -> Option<Amount> {
        self.0.checked_add(rhs.0).map(Amount)
    }

    pub fn checked_sub(self, rhs: Amount) -> Option<Amount> {
        self.0.checked_sub(rhs.0).map(Amount)
    }

    pub fn checked_mul(self, rhs: u128) -> Option<Amount> {
        self.0.checked_mul(rhs).map(Amount)
    }

    pub fn checked_div(self, rhs: u128) -> Option<Amount> {
        self.0.checked_div(rhs).map(Amount)
    }

    /// Multiply by a fraction (numerator / denominator) with overflow protection.
    pub fn mul_fraction(self, numerator: u128, denominator: u128) -> Option<Amount> {
        if denominator == 0 {
            return None;
        }
        // Use u128 arithmetic; for very large values this could overflow,
        // but for the scaffold this is sufficient.
        self.0
            .checked_mul(numerator)
            .and_then(|v| v.checked_div(denominator))
            .map(Amount)
    }
}

impl Add for Amount {
    type Output = Amount;

    fn add(self, rhs: Amount) -> Amount {
        Amount(self.0 + rhs.0)
    }
}

impl Sub for Amount {
    type Output = Amount;

    fn sub(self, rhs: Amount) -> Amount {
        Amount(self.0 - rhs.0)
    }
}

impl fmt::Display for Amount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<u64> for Amount {
    fn from(value: u64) -> Self {
        Self(value as u128)
    }
}

impl TryFrom<Amount> for u64 {
    type Error = TryFromIntError;

    /// Convert to u64, returning an error if the value exceeds `u64::MAX`.
    /// Use this when crossing the DeFi (u128) ↔ core4 (u64) boundary.
    fn try_from(amount: Amount) -> Result<u64, TryFromIntError> {
        u64::try_from(amount.0)
    }
}

/// A 32-byte address identifying an account in the system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Address(pub [u8; 32]);

impl Address {
    pub fn zero() -> Self {
        Self([0u8; 32])
    }

    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Display first 4 and last 4 bytes in hex for readability
        write!(f, "0x")?;
        for b in &self.0[..4] {
            write!(f, "{b:02x}")?;
        }
        write!(f, "...")?;
        for b in &self.0[28..] {
            write!(f, "{b:02x}")?;
        }
        Ok(())
    }
}

/// Unique identifier for a liquidity or lending pool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PoolId(pub u64);

impl PoolId {
    pub fn new(id: u64) -> Self {
        Self(id)
    }
}

impl fmt::Display for PoolId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "pool#{}", self.0)
    }
}

/// A price represented as a fraction (numerator / denominator) to avoid
/// floating-point precision issues in financial calculations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Price {
    pub numerator: u128,
    pub denominator: u128,
}

impl Price {
    /// Create a new price. Panics if denominator is zero.
    pub fn new(numerator: u128, denominator: u128) -> Self {
        assert!(denominator > 0, "Price denominator must be non-zero");
        Self {
            numerator,
            denominator,
        }
    }

    /// Create a price from an integer value (denominator = 1).
    pub fn from_integer(value: u128) -> Self {
        Self {
            numerator: value,
            denominator: 1,
        }
    }

    /// Apply this price to an amount: amount * (numerator / denominator).
    pub fn apply(&self, amount: Amount) -> Option<Amount> {
        amount.mul_fraction(self.numerator, self.denominator)
    }

    /// Return the inverse price (swap numerator and denominator).
    pub fn inverse(&self) -> Option<Price> {
        if self.numerator == 0 {
            return None;
        }
        Some(Price {
            numerator: self.denominator,
            denominator: self.numerator,
        })
    }
}

impl fmt::Display for Price {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.numerator, self.denominator)
    }
}

/// Unix timestamp in seconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Timestamp(pub u64);

impl Timestamp {
    pub fn new(secs: u64) -> Self {
        Self(secs)
    }

    pub fn as_secs(&self) -> u64 {
        self.0
    }

    /// Duration in seconds between two timestamps.
    pub fn elapsed_since(&self, earlier: Timestamp) -> u64 {
        self.0.saturating_sub(earlier.0)
    }
}

impl fmt::Display for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "t={}", self.0)
    }
}

/// Canonical liquidation event payload shared by Rust lending logic and
/// Solidity adapter conformance tests.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiquidationEvent {
    pub position_id: [u8; 32],
    pub collateral_seized: Amount,
    pub debt_repaid: Amount,
    pub liquidator: Address,
    pub timestamp: Timestamp,
}

impl LiquidationEvent {
    pub fn new(
        position_id: [u8; 32],
        collateral_seized: Amount,
        debt_repaid: Amount,
        liquidator: Address,
        timestamp: Timestamp,
    ) -> Self {
        Self {
            position_id,
            collateral_seized,
            debt_repaid,
            liquidator,
            timestamp,
        }
    }

    pub fn borrower_position_id(borrower: &Address) -> [u8; 32] {
        let mut position_id = [0u8; 32];
        position_id[12..].copy_from_slice(&borrower.as_bytes()[12..]);
        position_id
    }

    pub fn abi_encode(&self) -> Vec<u8> {
        let mut encoded = Vec::with_capacity(32 * 5);
        encoded.extend_from_slice(&self.position_id);
        encoded.extend_from_slice(&encode_u256(self.collateral_seized.0));
        encoded.extend_from_slice(&encode_u256(self.debt_repaid.0));
        encoded.extend_from_slice(&encode_address(&self.liquidator));
        encoded.extend_from_slice(&encode_u256(self.timestamp.as_secs() as u128));
        encoded
    }
}

fn encode_u256(value: u128) -> [u8; 32] {
    let mut out = [0u8; 32];
    out[16..].copy_from_slice(&value.to_be_bytes());
    out
}

fn encode_address(address: &Address) -> [u8; 32] {
    let mut out = [0u8; 32];
    out[12..].copy_from_slice(&address.as_bytes()[12..]);
    out
}

/// Utility signal snapshot used to weight DeFi incentives toward verified
/// forecast/report usefulness outcomes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct UtilitySignalSnapshot {
    /// Verified forecast usefulness score in basis points (0..=10_000).
    pub forecast_usefulness_bps: u16,
    /// Verified report usefulness score in basis points (0..=10_000).
    pub report_usefulness_bps: u16,
    /// Confidence in utility verification quality (0..=10_000).
    pub verification_confidence_bps: u16,
    /// Count of verified utility events in the sampling window.
    pub verified_events: u64,
    /// Count of unverified utility events in the sampling window.
    pub unverified_events: u64,
}

impl UtilitySignalSnapshot {
    pub const BPS_SCALE: u16 = 10_000;
    pub const NEUTRAL_BPS: u16 = 5_000;

    pub fn neutral() -> Self {
        Self {
            forecast_usefulness_bps: Self::NEUTRAL_BPS,
            report_usefulness_bps: Self::NEUTRAL_BPS,
            verification_confidence_bps: Self::BPS_SCALE,
            verified_events: 0,
            unverified_events: 0,
        }
    }

    pub fn clamped(self) -> Self {
        Self {
            forecast_usefulness_bps: self.forecast_usefulness_bps.min(Self::BPS_SCALE),
            report_usefulness_bps: self.report_usefulness_bps.min(Self::BPS_SCALE),
            verification_confidence_bps: self.verification_confidence_bps.min(Self::BPS_SCALE),
            verified_events: self.verified_events,
            unverified_events: self.unverified_events,
        }
    }

    /// Composite utility score in basis points. Forecast and report usefulness
    /// are averaged and scaled by verification confidence.
    pub fn composite_usefulness_bps(&self) -> u16 {
        let signal = self.clamped();
        let avg = (signal.forecast_usefulness_bps as u32 + signal.report_usefulness_bps as u32) / 2;
        let weighted = (avg * signal.verification_confidence_bps as u32) / Self::BPS_SCALE as u32;
        weighted as u16
    }

    /// Ratio of verified utility events in basis points. If no events are
    /// available, returns neutral to avoid over-penalizing cold-start pools.
    pub fn verified_event_ratio_bps(&self) -> u16 {
        let total = self.verified_events.saturating_add(self.unverified_events);
        if total == 0 {
            return Self::NEUTRAL_BPS;
        }
        let ratio = self
            .verified_events
            .saturating_mul(Self::BPS_SCALE as u64)
            .saturating_div(total);
        ratio.min(Self::BPS_SCALE as u64) as u16
    }
}

impl Default for UtilitySignalSnapshot {
    fn default() -> Self {
        Self::neutral()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_amount_zero() {
        assert!(Amount::ZERO.is_zero());
        assert!(Amount::new(0).is_zero());
        assert!(!Amount::new(1).is_zero());
    }

    #[test]
    fn test_liquidation_event_abi_encoding_layout() {
        let mut borrower_bytes = [0u8; 32];
        borrower_bytes[31] = 0xaa;
        let borrower = Address::from_bytes(borrower_bytes);

        let mut liquidator_bytes = [0u8; 32];
        liquidator_bytes[31] = 0xbb;
        let liquidator = Address::from_bytes(liquidator_bytes);

        let event = LiquidationEvent::new(
            LiquidationEvent::borrower_position_id(&borrower),
            Amount::new(210),
            Amount::new(200),
            liquidator,
            Timestamp::new(3),
        );

        let encoded = event.abi_encode();
        assert_eq!(encoded.len(), 160);
        assert_eq!(
            &encoded[..32],
            &LiquidationEvent::borrower_position_id(&borrower)
        );
        assert_eq!(encoded[63], 210);
        assert_eq!(encoded[95], 200);
        assert_eq!(encoded[127], 0xbb);
        assert_eq!(encoded[159], 3);
    }

    #[test]
    fn test_amount_checked_add() {
        let a = Amount::new(100);
        let b = Amount::new(200);
        assert_eq!(a.checked_add(b), Some(Amount::new(300)));
    }

    #[test]
    fn test_amount_checked_add_overflow() {
        let a = Amount::new(u128::MAX);
        let b = Amount::new(1);
        assert_eq!(a.checked_add(b), None);
    }

    #[test]
    fn test_amount_checked_sub() {
        let a = Amount::new(300);
        let b = Amount::new(100);
        assert_eq!(a.checked_sub(b), Some(Amount::new(200)));
    }

    #[test]
    fn test_amount_checked_sub_underflow() {
        let a = Amount::new(100);
        let b = Amount::new(200);
        assert_eq!(a.checked_sub(b), None);
    }

    #[test]
    fn test_amount_checked_mul() {
        assert_eq!(Amount::new(100).checked_mul(5), Some(Amount::new(500)));
    }

    #[test]
    fn test_amount_checked_div() {
        assert_eq!(Amount::new(100).checked_div(5), Some(Amount::new(20)));
        assert_eq!(Amount::new(100).checked_div(0), None);
    }

    #[test]
    fn test_amount_mul_fraction() {
        // 1000 * 30 / 10000 = 3
        assert_eq!(
            Amount::new(1000).mul_fraction(30, 10000),
            Some(Amount::new(3))
        );
        // denominator zero
        assert_eq!(Amount::new(1000).mul_fraction(1, 0), None);
    }

    #[test]
    fn test_amount_ordering() {
        assert!(Amount::new(100) < Amount::new(200));
        assert!(Amount::new(200) > Amount::new(100));
        assert_eq!(Amount::new(100), Amount::new(100));
    }

    #[test]
    fn test_amount_add_sub_ops() {
        let a = Amount::new(10);
        let b = Amount::new(5);
        assert_eq!(a + b, Amount::new(15));
        assert_eq!(a - b, Amount::new(5));
    }

    #[test]
    fn test_price_new() {
        let p = Price::new(3, 1);
        assert_eq!(p.numerator, 3);
        assert_eq!(p.denominator, 1);
    }

    #[test]
    #[should_panic(expected = "denominator must be non-zero")]
    fn test_price_zero_denominator() {
        Price::new(1, 0);
    }

    #[test]
    fn test_price_from_integer() {
        let p = Price::from_integer(5);
        assert_eq!(p.apply(Amount::new(100)), Some(Amount::new(500)));
    }

    #[test]
    fn test_price_apply() {
        // price = 3/2, apply to 100 = 100 * 3 / 2 = 150
        let p = Price::new(3, 2);
        assert_eq!(p.apply(Amount::new(100)), Some(Amount::new(150)));
    }

    #[test]
    fn test_price_inverse() {
        let p = Price::new(3, 2);
        let inv = p.inverse().unwrap();
        assert_eq!(inv.numerator, 2);
        assert_eq!(inv.denominator, 3);
    }

    #[test]
    fn test_price_inverse_zero() {
        let p = Price {
            numerator: 0,
            denominator: 1,
        };
        assert!(p.inverse().is_none());
    }

    #[test]
    fn test_address_zero() {
        let addr = Address::zero();
        assert_eq!(addr.0, [0u8; 32]);
    }

    #[test]
    fn test_address_display() {
        let mut bytes = [0u8; 32];
        bytes[0] = 0xAB;
        bytes[31] = 0xCD;
        let addr = Address::from_bytes(bytes);
        let display = format!("{addr}");
        assert!(display.starts_with("0x"));
        assert!(display.contains("..."));
    }

    #[test]
    fn test_asset_id() {
        let id = AssetId::new("WRF");
        assert_eq!(id.as_str(), "WRF");
        assert_eq!(format!("{id}"), "WRF");
    }

    #[test]
    fn test_pool_id_display() {
        let pid = PoolId::new(42);
        assert_eq!(format!("{pid}"), "pool#42");
    }

    #[test]
    fn test_timestamp_elapsed_since() {
        let t1 = Timestamp::new(100);
        let t2 = Timestamp::new(150);
        assert_eq!(t2.elapsed_since(t1), 50);
        assert_eq!(t1.elapsed_since(t2), 0); // saturating
    }

    #[test]
    fn test_utility_signal_default_is_neutral() {
        let signal = UtilitySignalSnapshot::default();
        assert_eq!(signal.composite_usefulness_bps(), 5_000);
        assert_eq!(signal.verified_event_ratio_bps(), 5_000);
    }

    #[test]
    fn test_wrf_units_per_coin() {
        assert_eq!(WRF_UNITS_PER_COIN, 100_000_000);
        // 1 WRF coin in base units
        let one_coin = Amount::new(WRF_UNITS_PER_COIN);
        assert_eq!(one_coin.0, 100_000_000);
    }

    #[test]
    fn test_amount_from_u64() {
        let a: Amount = Amount::from(42u64);
        assert_eq!(a.0, 42);
        let b: Amount = Amount::from(u64::MAX);
        assert_eq!(b.0, u64::MAX as u128);
    }

    #[test]
    fn test_amount_try_into_u64() {
        let small = Amount::new(12345);
        let val: u64 = small.try_into().unwrap();
        assert_eq!(val, 12345);

        let max = Amount::new(u64::MAX as u128);
        let val: u64 = max.try_into().unwrap();
        assert_eq!(val, u64::MAX);
    }

    #[test]
    fn test_amount_try_into_u64_overflow() {
        let too_big = Amount::new(u64::MAX as u128 + 1);
        let result: Result<u64, _> = too_big.try_into();
        assert!(result.is_err());
    }

    #[test]
    fn test_utility_signal_ratio_uses_verified_vs_unverified() {
        let signal = UtilitySignalSnapshot {
            forecast_usefulness_bps: 9_000,
            report_usefulness_bps: 8_000,
            verification_confidence_bps: 10_000,
            verified_events: 9,
            unverified_events: 1,
        };
        assert_eq!(signal.verified_event_ratio_bps(), 9_000);
        assert_eq!(signal.composite_usefulness_bps(), 8_500);
    }
}
