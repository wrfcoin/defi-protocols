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
