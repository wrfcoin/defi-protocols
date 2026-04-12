use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use std::fmt;
use wrfcoin_defi_common::types::Address;

pub const LENDING_INTERFACE_V0: &str = "lending-interface-v0";
const ACCOUNT_PREFIX: &str = "lending:";
const ADDRESS_HEX_LEN: usize = 64;
const MAX_BPS: u16 = 10_000;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct LendingAccountId(pub String);

impl LendingAccountId {
    pub fn from_address(address: Address) -> Self {
        Self(format!(
            "{ACCOUNT_PREFIX}{}",
            bytes_to_hex(address.as_bytes())
        ))
    }

    pub fn to_address(&self) -> Result<Address, WireSchemaError> {
        let raw = self.0.strip_prefix(ACCOUNT_PREFIX).ok_or_else(|| {
            WireSchemaError::InvalidAccountId {
                reason: "missing lending: prefix".into(),
            }
        })?;

        if raw.len() != ADDRESS_HEX_LEN {
            return Err(WireSchemaError::InvalidAccountId {
                reason: "address payload must be 64 lowercase hex characters".into(),
            });
        }

        let mut out = [0u8; 32];
        for (index, chunk) in raw.as_bytes().chunks(2).enumerate() {
            out[index] =
                decode_hex_byte(chunk).ok_or_else(|| WireSchemaError::InvalidAccountId {
                    reason: "address payload contains non-hex bytes".into(),
                })?;
        }
        Ok(Address(out))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecimalString(#[serde(deserialize_with = "deserialize_decimal_string")] pub String);

impl DecimalString {
    pub fn new(value: impl Into<String>) -> Result<Self, WireSchemaError> {
        let value = value.into();
        validate_decimal_string(&value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct RiskBps(#[serde(deserialize_with = "deserialize_risk_bps")] pub u16);

impl RiskBps {
    pub fn new(value: u16) -> Result<Self, WireSchemaError> {
        if value > MAX_BPS {
            return Err(WireSchemaError::InvalidRiskBps { value });
        }
        Ok(Self(value))
    }

    pub fn get(self) -> u16 {
        self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LendingWirePositionV0 {
    pub schema_version: String,
    pub account_id: LendingAccountId,
    pub principal: DecimalString,
    pub collateral_value: DecimalString,
    pub oracle_price: DecimalString,
    pub max_ltv_bps: RiskBps,
    pub liquidation_threshold_bps: RiskBps,
    pub liquidation_penalty_bps: RiskBps,
}

impl LendingWirePositionV0 {
    pub fn new(
        account_id: LendingAccountId,
        principal: DecimalString,
        collateral_value: DecimalString,
        oracle_price: DecimalString,
        max_ltv_bps: RiskBps,
        liquidation_threshold_bps: RiskBps,
        liquidation_penalty_bps: RiskBps,
    ) -> Self {
        Self {
            schema_version: LENDING_INTERFACE_V0.into(),
            account_id,
            principal,
            collateral_value,
            oracle_price,
            max_ltv_bps,
            liquidation_threshold_bps,
            liquidation_penalty_bps,
        }
    }

    pub fn validate(&self) -> Result<(), WireSchemaError> {
        if self.schema_version != LENDING_INTERFACE_V0 {
            return Err(WireSchemaError::InvalidSchemaVersion {
                version: self.schema_version.clone(),
            });
        }
        if self.liquidation_threshold_bps.get() < self.max_ltv_bps.get() {
            return Err(WireSchemaError::InvalidRiskRelationship {
                reason: "liquidation threshold must be >= max LTV".into(),
            });
        }
        self.account_id.to_address()?;
        validate_decimal_string(self.principal.as_str())?;
        validate_decimal_string(self.collateral_value.as_str())?;
        validate_decimal_string(self.oracle_price.as_str())?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WireSchemaError {
    #[error("invalid schema version: {version}")]
    InvalidSchemaVersion { version: String },

    #[error("invalid account id: {reason}")]
    InvalidAccountId { reason: String },

    #[error("invalid decimal string: {reason}")]
    InvalidDecimalString { reason: String },

    #[error("invalid risk bps: {value}")]
    InvalidRiskBps { value: u16 },

    #[error("invalid risk relationship: {reason}")]
    InvalidRiskRelationship { reason: String },
}

fn deserialize_decimal_string<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    struct DecimalVisitor;

    impl<'de> Visitor<'de> for DecimalVisitor {
        type Value = String;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a decimal string with digits and at most one decimal point")
        }

        fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            validate_decimal_string(value).map_err(E::custom)?;
            Ok(value.to_string())
        }
    }

    deserializer.deserialize_str(DecimalVisitor)
}

fn deserialize_risk_bps<'de, D>(deserializer: D) -> Result<u16, D::Error>
where
    D: Deserializer<'de>,
{
    let value = u16::deserialize(deserializer)?;
    RiskBps::new(value)
        .map(|risk| risk.0)
        .map_err(de::Error::custom)
}

fn validate_decimal_string(value: &str) -> Result<(), WireSchemaError> {
    if value.is_empty() {
        return Err(WireSchemaError::InvalidDecimalString {
            reason: "value must not be empty".into(),
        });
    }
    if value.starts_with('.') || value.ends_with('.') {
        return Err(WireSchemaError::InvalidDecimalString {
            reason: "value must not start or end with a decimal point".into(),
        });
    }

    let mut dot_count = 0;
    for ch in value.chars() {
        if ch == '.' {
            dot_count += 1;
            if dot_count > 1 {
                return Err(WireSchemaError::InvalidDecimalString {
                    reason: "value must contain at most one decimal point".into(),
                });
            }
        } else if !ch.is_ascii_digit() {
            return Err(WireSchemaError::InvalidDecimalString {
                reason: "value must contain only ASCII digits and an optional decimal point".into(),
            });
        }
    }

    Ok(())
}

fn bytes_to_hex(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn decode_hex_byte(two_chars: &[u8]) -> Option<u8> {
    if two_chars.len() != 2 {
        return None;
    }
    let high = decode_nibble(two_chars[0])?;
    let low = decode_nibble(two_chars[1])?;
    Some((high << 4) | low)
}

fn decode_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn address(byte: u8) -> Address {
        Address([byte; 32])
    }

    #[test]
    fn account_id_round_trips_through_wire_string() {
        let addr = address(0x2a);
        let wire = LendingAccountId::from_address(addr);

        assert_eq!(
            wire.to_address()
                .expect("round-trip address decode should succeed"),
            addr
        );
        assert_eq!(wire.0.len(), ACCOUNT_PREFIX.len() + ADDRESS_HEX_LEN);
    }

    #[test]
    fn invalid_account_id_is_rejected() {
        let invalid = LendingAccountId("not-a-wire-id".into());
        assert!(matches!(
            invalid.to_address(),
            Err(WireSchemaError::InvalidAccountId { .. })
        ));
    }

    #[test]
    fn decimal_string_constructor_rejects_bad_values() {
        assert!(DecimalString::new("123.45").is_ok());
        assert!(DecimalString::new("00123").is_ok());
        assert!(DecimalString::new("12.3.4").is_err());
        assert!(DecimalString::new("-12").is_err());
        assert!(DecimalString::new("abc").is_err());
    }

    #[test]
    fn serde_round_trip_preserves_wire_schema() {
        let payload = LendingWirePositionV0::new(
            LendingAccountId::from_address(address(0x44)),
            DecimalString::new("1000.50").expect("valid decimal string"),
            DecimalString::new("1500.75").expect("valid decimal string"),
            DecimalString::new("1.2500").expect("valid decimal string"),
            RiskBps::new(6500).expect("valid risk bps"),
            RiskBps::new(8000).expect("valid risk bps"),
            RiskBps::new(500).expect("valid risk bps"),
        );

        let json = serde_json::to_string(&payload).expect("serialization should succeed");
        let round_trip: LendingWirePositionV0 =
            serde_json::from_str(&json).expect("deserialization should succeed");

        round_trip
            .validate()
            .expect("round-tripped payload should be valid");
        assert_eq!(round_trip, payload);
    }

    #[test]
    fn serde_validation_rejects_bad_decimal_and_bps_fields() {
        let bad_decimal = r#"{
            "schema_version":"lending-interface-v0",
            "account_id":"lending:1111111111111111111111111111111111111111111111111111111111111111",
            "principal":"10..5",
            "collateral_value":"1500.75",
            "oracle_price":"1.2500",
            "max_ltv_bps":6500,
            "liquidation_threshold_bps":8000,
            "liquidation_penalty_bps":500
        }"#;
        assert!(serde_json::from_str::<LendingWirePositionV0>(bad_decimal).is_err());

        let bad_bps = r#"{
            "schema_version":"lending-interface-v0",
            "account_id":"lending:1111111111111111111111111111111111111111111111111111111111111111",
            "principal":"100.5",
            "collateral_value":"1500.75",
            "oracle_price":"1.2500",
            "max_ltv_bps":12000,
            "liquidation_threshold_bps":8000,
            "liquidation_penalty_bps":500
        }"#;
        assert!(serde_json::from_str::<LendingWirePositionV0>(bad_bps).is_err());
    }

    #[test]
    fn validate_rejects_invalid_risk_relationships() {
        let payload = LendingWirePositionV0::new(
            LendingAccountId::from_address(address(0x55)),
            DecimalString::new("100").expect("valid decimal string"),
            DecimalString::new("150").expect("valid decimal string"),
            DecimalString::new("1.5").expect("valid decimal string"),
            RiskBps::new(7000).expect("valid risk bps"),
            RiskBps::new(6500).expect("valid risk bps"),
            RiskBps::new(500).expect("valid risk bps"),
        );

        assert!(matches!(
            payload.validate(),
            Err(WireSchemaError::InvalidRiskRelationship { .. })
        ));
    }
}
