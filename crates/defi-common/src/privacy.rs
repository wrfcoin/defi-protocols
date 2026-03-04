use crate::error::{DefiError, DefiResult};
use crate::types::{Amount, PoolId, Timestamp};
use serde::{Deserialize, Serialize};

/// Participant class used for plain-language policy statements.
///
/// Equal-liability policy applies to all variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ParticipantClass {
    Human,
    AiAgent,
    Robot,
}

/// Hashed participant identifier. Must never be a raw address or PII.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct OpaqueParticipantId(pub String);

impl OpaqueParticipantId {
    pub fn is_valid_hash(&self) -> bool {
        let trimmed = self.0.trim();
        !trimmed.is_empty()
            && trimmed == self.0
            && trimmed.len() <= 128
            && trimmed
                .bytes()
                .all(|c| c.is_ascii_hexdigit() || c == b'-' || c == b'_')
    }
}

/// Privacy policy for utility reward aggregation and disclosure flow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrivacyRewardFlowPolicy {
    /// Policy version for deterministic compatibility.
    pub policy_version: u16,
    /// Minimum cohort size before payouts can be aggregated and emitted.
    pub min_aggregation_cohort: u16,
    /// Maximum selective-disclosure scopes per disclosure request.
    pub max_disclosure_scope_entries: u16,
    /// Maximum retention window for raw utility metadata.
    pub metadata_retention_blocks: u64,
}

impl Default for PrivacyRewardFlowPolicy {
    fn default() -> Self {
        Self {
            policy_version: 1,
            min_aggregation_cohort: 16,
            max_disclosure_scope_entries: 8,
            metadata_retention_blocks: 20_000,
        }
    }
}

impl PrivacyRewardFlowPolicy {
    pub fn validate(&self) -> DefiResult<()> {
        if self.policy_version == 0 {
            return Err(DefiError::InvalidAmount {
                reason: "policy_version must be >= 1".into(),
            });
        }
        if self.min_aggregation_cohort < 2 {
            return Err(DefiError::InvalidAmount {
                reason: "min_aggregation_cohort must be >= 2".into(),
            });
        }
        if self.max_disclosure_scope_entries == 0 {
            return Err(DefiError::InvalidAmount {
                reason: "max_disclosure_scope_entries must be >= 1".into(),
            });
        }
        if self.metadata_retention_blocks == 0 {
            return Err(DefiError::InvalidAmount {
                reason: "metadata_retention_blocks must be >= 1".into(),
            });
        }
        Ok(())
    }
}

/// Privacy-safe utility reward signal input used for batch aggregation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RewardAggregationInput {
    pub participant_id: OpaqueParticipantId,
    pub participant_class: ParticipantClass,
    pub pool_id: PoolId,
    pub utility_score_bps: u16,
    pub reward_amount: Amount,
}

/// Minimal metadata envelope that may be persisted outside shielded payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MinimalMetadataEnvelope {
    pub policy_version: u16,
    pub pool_id: PoolId,
    pub utility_score_bucket: u16,
    pub participant_count: u32,
    pub emission_epoch: u64,
}

/// Aggregated reward batch commitment emitted prior to shielding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AggregatedRewardBatch {
    pub batch_id: String,
    pub reward_commitment: String,
    pub total_reward: Amount,
    pub participant_count: u32,
    pub metadata: MinimalMetadataEnvelope,
    pub created_at: Timestamp,
}

/// Shielded batch reference published after reward shielding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShieldedRewardBatchRef {
    pub batch_id: String,
    pub shielded_commitment: String,
    pub nullifier: String,
    pub encrypted_distribution_ref: String,
}

/// Selective disclosure request for compliance-safe audits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectiveDisclosureRequest {
    pub batch_id: String,
    pub scope: Vec<String>,
    pub expires_at_block: u64,
    pub requester_id: String,
}

/// Disclosure result emitted by the disclosure adapter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisclosureResult {
    pub batch_id: String,
    pub disclosure_hash: String,
    pub disclosed_fields: Vec<String>,
    pub attestation_hash: String,
}

/// Reference interface for aggregation hook implementations.
pub trait PrivacyRewardAggregator {
    fn build_aggregated_batch(
        &self,
        inputs: &[RewardAggregationInput],
        policy: PrivacyRewardFlowPolicy,
        created_at: Timestamp,
    ) -> DefiResult<AggregatedRewardBatch>;
}

/// Reference interface for shielded payout adapters.
pub trait RewardShieldingAdapter {
    fn shield_batch(
        &self,
        batch: &AggregatedRewardBatch,
        policy: PrivacyRewardFlowPolicy,
    ) -> DefiResult<ShieldedRewardBatchRef>;

    fn verify_shielded_batch_ref(&self, reference: &ShieldedRewardBatchRef) -> DefiResult<()>;
}

/// Reference interface for selective disclosure adapters.
pub trait RewardSelectiveDisclosureAdapter {
    fn disclose(
        &self,
        request: &SelectiveDisclosureRequest,
        policy: PrivacyRewardFlowPolicy,
    ) -> DefiResult<DisclosureResult>;
}

/// Metadata leakage telemetry sample for reward-flow risk scoring.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct MetadataLeakageTelemetry {
    /// Count of raw participant identifier exposures.
    pub raw_participant_identifier_exposures: u64,
    /// Count of raw amount disclosures (not bucketized).
    pub raw_amount_exposures: u64,
    /// Count of timestamp granularity leaks finer than policy allows.
    pub high_granularity_timestamp_exposures: u64,
    /// Count of scope over-disclosures (fields beyond approved scope).
    pub out_of_scope_disclosures: u64,
    /// Total reward flow events observed in the measurement window.
    pub total_reward_flow_events: u64,
}

/// Compute metadata leakage risk score in basis points (0 low risk .. 10_000 high risk).
pub fn metadata_leakage_risk_score_bps(sample: MetadataLeakageTelemetry) -> u16 {
    if sample.total_reward_flow_events == 0 {
        return 0;
    }
    let denom = sample.total_reward_flow_events as u128;
    let normalize = |count: u64| -> u128 {
        (count as u128)
            .saturating_mul(10_000u128)
            .saturating_div(denom)
            .min(10_000)
    };

    let id_risk = normalize(sample.raw_participant_identifier_exposures);
    let amount_risk = normalize(sample.raw_amount_exposures);
    let ts_risk = normalize(sample.high_granularity_timestamp_exposures);
    let scope_risk = normalize(sample.out_of_scope_disclosures);

    // Weighted risk model (must sum to 10_000 bps).
    let weighted = id_risk.saturating_mul(3_500)
        + amount_risk.saturating_mul(2_500)
        + ts_risk.saturating_mul(2_000)
        + scope_risk.saturating_mul(2_000);

    (weighted / 10_000).min(10_000) as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_validation_rejects_invalid_bounds() {
        let bad = PrivacyRewardFlowPolicy {
            min_aggregation_cohort: 1,
            ..PrivacyRewardFlowPolicy::default()
        };
        assert!(bad.validate().is_err());
    }

    #[test]
    fn opaque_participant_id_accepts_hash_like_strings() {
        let id = OpaqueParticipantId("abc123ff-001".to_string());
        assert!(id.is_valid_hash());
    }

    #[test]
    fn leakage_risk_score_increases_with_more_exposures() {
        let low = metadata_leakage_risk_score_bps(MetadataLeakageTelemetry {
            raw_participant_identifier_exposures: 1,
            raw_amount_exposures: 1,
            high_granularity_timestamp_exposures: 1,
            out_of_scope_disclosures: 1,
            total_reward_flow_events: 1_000,
        });
        let high = metadata_leakage_risk_score_bps(MetadataLeakageTelemetry {
            raw_participant_identifier_exposures: 120,
            raw_amount_exposures: 110,
            high_granularity_timestamp_exposures: 100,
            out_of_scope_disclosures: 90,
            total_reward_flow_events: 1_000,
        });
        assert!(high > low);
    }
}
