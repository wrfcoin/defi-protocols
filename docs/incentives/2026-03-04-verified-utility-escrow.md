# Verified-Utility Reward Escrow Policy (defi-protocols#9)

Date: 2026-03-04

## Objective

Prevent immediate reward extraction before quality/utility verification settles.

## Implementation

Escrow implementation lives in:

- `crates/yield-farming/src/escrow.rs`
- `crates/yield-farming/src/farm.rs` (`harvest_with_outcome`, `harvest_with_signal_source`)

## Policy

`RewardEscrowPolicy` controls reward release behavior:

- `min_quality_score_bps`: minimum verified quality threshold required for release.
- `forfeit_on_low_quality`: forfeit rewards if quality verification settles below threshold.
- `forfeit_on_fraud`: forfeit rewards on fraud outcome.

## Release And Forfeit Conditions

1. `Pending` verification outcome: reward is held in escrow.
2. `Verified { quality_score_bps >= min_quality_score_bps }`: reward is released.
3. `Verified { quality_score_bps < min_quality_score_bps }` with `forfeit_on_low_quality = true`:
   reward is forfeited.
4. `FraudDetected` with `forfeit_on_fraud = true`: reward is forfeited.

## Telemetry KPI

`EscrowTelemetry` tracks:

- `total_rewards_observed`
- `rewards_released`
- `rewards_released_after_verification`
- `rewards_forfeited`
- `rewards_currently_escrowed`
- `settled_verification_events`

Primary KPI:

- `released_after_verification_share_bps()`

This is the share of released rewards that were released only after verification completion.

## Integration Test Coverage

`crates/yield-farming/src/farm.rs` and `crates/yield-farming/src/escrow.rs` tests cover:

- pending outcome -> escrow hold -> later verified release
- low-quality outcome forfeits
- fraud outcome forfeits

## UX Tradeoff

Escrow introduces payout delay while quality verification settles. This reduces immediate liquidity
for participants but lowers incentives for low-quality or synthetic submissions.
