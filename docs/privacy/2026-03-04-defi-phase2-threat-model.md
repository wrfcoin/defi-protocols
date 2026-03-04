# Threat Model: Utility Reward Privacy Flow (defi-protocols#8)

Date: 2026-03-04

## Assets

- Participant identities and participation patterns
- Utility contribution signals
- Reward amount allocations before shielding
- Disclosure records and attestation artifacts

## Trust Boundaries

1. Utility ingestion boundary: external utility evidence into reward service
2. Aggregation boundary: participant-level points to batched commitments
3. Shielding boundary: batch commitments to shielded payout system
4. Disclosure boundary: exceptional audit access path
5. Telemetry boundary: operational metrics and dashboards

## Threats and Controls

| Threat | Description | Control(s) | Residual Risk |
| --- | --- | --- | --- |
| T1: Linkability through identifiers | Adversary correlates reward entries to participants. | `OpaqueParticipantId` requirement, cohort aggregation, shielded commitments. | Medium if upstream service emits unhashed IDs. |
| T2: Amount inference from clear metadata | Exact reward amounts leak behavior patterns. | Expose only bucketized/aggregate metadata in `MinimalMetadataEnvelope`. | Medium-low if bucket sizing is too fine. |
| T3: Timing correlation | Fine timestamps reveal participant actions. | Coarse `emission_epoch`; treat fine-grained timestamps as leakage telemetry. | Medium in low-volume pools. |
| T4: Disclosure overreach | Audit pathway discloses unapproved fields. | Scope-limited disclosure requests, max scope entries, expiration bounds, attestation hash output. | Medium if policy exceptions become frequent. |
| T5: Cohort collapse | Small cohorts deanonymize participants. | `min_aggregation_cohort` policy validation and enforcement in aggregators. | High for low-liquidity pools without fallback batching. |
| T6: Policy drift between systems | DeFi adapters and contracts apply different privacy rules. | Versioned `PrivacyRewardFlowPolicy`, explicit adapter traits, release conformance checks. | Medium until full integration tests exist. |

## Abuse Cases

- Repeated targeted disclosure requests to reconstruct participant history.
- Combining off-chain utility events with reward emission timing.
- Internal operator extracting raw identifiers before shielding handoff.

## Security Requirements

- Disclosure adapters MUST validate scope count and expiration.
- Aggregators MUST reject batches below policy cohort threshold.
- Systems MUST continuously measure:
  - raw identifier exposures
  - raw amount exposures
  - high-granularity timestamp exposures
  - out-of-scope disclosures

## KPI and Gates

- Primary KPI: `metadata_leakage_risk_score_bps`
- Release gate recommendation:
  - block release if KPI exceeds agreed threshold or regresses materially release-over-release

## Open Risks

- Core4 default-mode privacy decisions may require API/format changes.
- Existing contract interfaces may need additional fields for shielded references and attestations.
