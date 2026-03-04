# Data Minimization Requirements (defi-protocols#8)

Date: 2026-03-04

## Objective

Define explicit data minimization requirements for utility-linked DeFi reward flows.

## Requirements

| ID | Requirement | Control | Verification |
| --- | --- | --- | --- |
| DM-01 | Participant identifiers MUST be opaque/hashes in reward-flow inputs. | `OpaqueParticipantId` only; no raw wallet or account identifiers in `RewardAggregationInput`. | Unit check: `OpaqueParticipantId::is_valid_hash()` and adapter integration tests. |
| DM-02 | Raw participant utility traces MUST NOT be stored in clear metadata envelopes. | `MinimalMetadataEnvelope` only exposes aggregate/bucket fields. | Schema review and serialization snapshot tests. |
| DM-03 | Reward processing MUST enforce a minimum aggregation cohort. | `PrivacyRewardFlowPolicy.min_aggregation_cohort >= 2`. | `PrivacyRewardFlowPolicy::validate()` and adapter tests. |
| DM-04 | Clear-text metadata retention MUST be bounded. | `metadata_retention_blocks` policy control. | Runtime retention config audit and storage TTL checks. |
| DM-05 | Selective disclosure MUST be scope-limited and time-bounded. | `SelectiveDisclosureRequest.scope` + `expires_at_block`; max scope entries. | Disclosure adapter conformance tests. |
| DM-06 | Timestamp resolution SHOULD be coarse for non-shielded records. | Store only coarse `emission_epoch` in clear metadata. | Metadata schema lint and telemetry trend checks. |
| DM-07 | Raw amount and identity exposures MUST be measured continuously. | `MetadataLeakageTelemetry` + risk KPI function. | Scheduled risk simulation and telemetry dashboard. |
| DM-08 | Policy treatment MUST remain equivalent across humans, AI agents, and robots when severity is equivalent. | Class-agnostic privacy controls with `ParticipantClass` only for policy transparency. | Equal-severity scenario tests in simulation. |

## Allowed vs Restricted Fields

Allowed in clear metadata:
- `policy_version`
- `pool_id`
- `utility_score_bucket`
- `participant_count`
- `emission_epoch`

Restricted to shielded/encrypted paths:
- participant-level utility scores
- participant-linked reward amounts
- raw participant identifiers
- detailed event timestamps

## Operational Guardrails

- Reject policy configuration where retention or scope limits are missing.
- Track metadata leakage KPI every release cycle.
- Block rollout if leakage KPI regresses above agreed risk threshold.
