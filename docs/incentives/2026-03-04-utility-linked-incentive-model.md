# Utility-Linked Incentive Model (defi-protocols#7)

Date: 2026-03-04

## Objective

Weight DeFi rewards toward verified forecast/report usefulness instead of purely speculative activity.

## Model

Implementation lives in:
- `crates/yield-farming/src/utility_hooks.rs`
- `crates/yield-farming/src/farm.rs`
- shared utility signal type: `crates/defi-common/src/types.rs` (`UtilitySignalSnapshot`)

### Signal Inputs

- `forecast_usefulness_bps` (0..10,000)
- `report_usefulness_bps` (0..10,000)
- `verification_confidence_bps` (0..10,000)
- `verified_events` and `unverified_events`

### Reward Hook

1. Build composite utility score from forecast/report usefulness and confidence.
2. Blend composite score with verified-event ratio (weighted bps config).
3. Convert aggregate score to a bounded multiplier:
- below neutral score => penalty down to `min_multiplier_bps`
- above neutral score => boost up to `max_multiplier_bps`
4. Apply multiplier to baseline pool reward before reward-per-share accounting.

## Integration Points

### 1) core4 utility scoring feed

- Map core utility scoring outputs into `UtilitySignalSnapshot` per pool/epoch.
- Deliver via indexer or API adaptor into DeFi services that call:
  - `FarmManager::set_pool_utility_signal(pool_index, signal)`.

### 2) backend telemetry feed

- Backend telemetry should periodically emit verified utility event counts and confidence.
- DeFi runtime consumes this data to refresh pool-level utility signals.

### 3) reward telemetry KPI

- Use `UtilityBacktestTelemetry.utility_reward_ratio_bps` as the primary KPI:
  - ratio of weighted rewards that are utility-linked vs financial-only activity.
- For post-issue-#9 payout integrity, pair with escrow KPI:
  - `EscrowTelemetry.released_after_verification_share_bps()`.

## Backtest/Simulation

Included:
- Rust backtest telemetry support:
  - `UtilityRewardHook::run_backtest`
  - test: `backtest_tracks_utility_reward_ratio`
- CLI simulation script:
  - `python3 scripts/simulate_utility_incentives.py`

The script outputs baseline totals, weighted totals, and
`utility_reward_ratio_bps` for a deterministic scenario mix.
