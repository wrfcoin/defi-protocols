use serde::{Deserialize, Serialize};
use std::fmt;
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
            write!(f, "{:02x}", b)?;
        }
        write!(f, "...")?;
        for b in &self.0[28..] {
            write!(f, "{:02x}", b)?;
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
        assert_eq!(Amount::new(1000).mul_fraction(30, 10000), Some(Amount::new(3)));
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
        let p = Price { numerator: 0, denominator: 1 };
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
        let display = format!("{}", addr);
        assert!(display.starts_with("0x"));
        assert!(display.contains("..."));
    }

    #[test]
    fn test_asset_id() {
        let id = AssetId::new("WRF");
        assert_eq!(id.as_str(), "WRF");
        assert_eq!(format!("{}", id), "WRF");
    }

    #[test]
    fn test_pool_id_display() {
        let pid = PoolId::new(42);
        assert_eq!(format!("{}", pid), "pool#42");
    }

    #[test]
    fn test_timestamp_elapsed_since() {
        let t1 = Timestamp::new(100);
        let t2 = Timestamp::new(150);
        assert_eq!(t2.elapsed_since(t1), 50);
        assert_eq!(t1.elapsed_since(t2), 0); // saturating
    }
}
