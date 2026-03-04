# Privacy-Preserving Utility Reward Flow (defi-protocols#8)

Date: 2026-03-04

## Objective

Define a privacy-preserving reward pipeline for utility-linked DeFi incentives so useful behavior can
be rewarded without exposing participant-level activity.

## Scope

- Utility signal aggregation for reward computation
- Shielding handoff for payout commitments
- Selective disclosure boundaries for audits/compliance
- Metadata leakage telemetry for ongoing privacy risk measurement

## Reference Interfaces Delivered

Implemented in `crates/defi-common/src/privacy.rs` and exported via
`crates/defi-common/src/lib.rs`.

- Policy:
  - `PrivacyRewardFlowPolicy` with `validate()`
- Participant abstraction:
  - `ParticipantClass` (human, AI agent, robot)
  - `OpaqueParticipantId` (hashed identifier only)
- Aggregation types:
  - `RewardAggregationInput`
  - `MinimalMetadataEnvelope`
  - `AggregatedRewardBatch`
- Shielding + disclosure types:
  - `ShieldedRewardBatchRef`
  - `SelectiveDisclosureRequest`
  - `DisclosureResult`
- Adapter traits:
  - `PrivacyRewardAggregator`
  - `RewardShieldingAdapter`
  - `RewardSelectiveDisclosureAdapter`
- Telemetry + KPI:
  - `MetadataLeakageTelemetry`
  - `metadata_leakage_risk_score_bps(...)`

## Flow Pattern

1. Ingest utility-linked reward points as `RewardAggregationInput` with an `OpaqueParticipantId`.
2. Aggregate into a cohort-limited `AggregatedRewardBatch` using policy controls:
   - `min_aggregation_cohort`
   - `metadata_retention_blocks`
3. Persist only `MinimalMetadataEnvelope` in clear-text systems.
4. Pass batch into `RewardShieldingAdapter::shield_batch` and publish only
   `ShieldedRewardBatchRef`.
5. Handle exceptional audits via `RewardSelectiveDisclosureAdapter::disclose`, bounded by
   `max_disclosure_scope_entries` and expiration.
6. Emit `MetadataLeakageTelemetry` events and derive privacy KPI using
   `metadata_leakage_risk_score_bps`.

## Minimal Metadata Contract

Allowed clear metadata:
- policy version
- pool id
- utility score bucket (not raw per-participant score)
- participant count
- coarse emission epoch

Disallowed clear metadata:
- raw participant identity/address
- unbucketized reward amounts tied to a participant
- high-granularity timestamps tied to a participant

## Human-Readable Policy Impact

Equal-liability handling is preserved for humans, AI agents, and robots:
- `ParticipantClass` is retained for plain-language policy compatibility.
- Privacy controls are class-agnostic.
- If offense severity is equivalent, handling remains equivalent.

## Dependencies and Integration Risk

Depends on core4 privacy default-mode decisions for:
- accepted shielded commitment format
- disclosure attestation format

Known integration risk:
- additional adapter coordination between DeFi reward services and existing contract interfaces.
