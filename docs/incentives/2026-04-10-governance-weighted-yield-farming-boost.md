# Governance-Weighted Yield Farming Boost (defi-protocols#43)

Date: 2026-04-10

## Goal

Allow governance to favor higher-priority pools without increasing total protocol emissions and
without adding a new multiplicative reward layer on top of the reward-stacking risk identified in
`defi-protocols#47`.

## Design

Implementation lives in:

- `crates/yield-farming/src/governance.rs`

The governance boost is implemented as a routing layer over a fixed emission budget:

```text
delta_i = governance_score_i - neutral_score
multiplier_i = 1 + beta * delta_i / neutral_score
raw_weight_i = alloc_point_i * multiplier_i
normalized_share_i = raw_weight_i / sum(raw_weight_j)
pool_reward_i = emission_budget * normalized_share_i
```

Where:

- `neutral_score = 5_000 bps`
- `beta = 2_000 bps` by default, so governance can move a pool between `0.8x` and `1.2x`
  of its base routing weight before normalization

## Why This Does Not Amplify `#47`

The key safety rule is that governance never multiplies `rewardPerSecond` directly. Instead, it
only changes how a pre-existing emission budget is split across pools.

That gives the invariant:

```text
sum(pool_reward_i) = emission_budget
```

The implementation preserves that invariant exactly, including integer rounding, by distributing
leftover base units to the highest remainders after normalization.

## Worked Example

Two pools with equal base allocation points:

- Pool A governance score: `8_000`
- Pool B governance score: `2_000`
- `beta = 0.20`

Then:

```text
multiplier_A = 1.12x
multiplier_B = 0.88x
raw_weight_A = 112
raw_weight_B = 88
normalized_share_A = 56%
normalized_share_B = 44%
```

If the global emission budget is `1_000`, governance routes:

- Pool A: `560`
- Pool B: `440`

Total emissions remain `1_000`.

## Explicit Anti-Pattern

This issue intentionally avoids the unsafe formula:

```text
pool_reward_i = base_pool_reward_i * governance_multiplier_i
```

That approach quietly expands emissions whenever most pools have above-neutral governance scores.
For example, two equal pools with `9_000` governance scores would turn a `1_000` budget into
`1_160` under naive multiplication. The normalized router keeps the sum at `1_000`.

## Tests

The Rust unit tests cover:

- neutral scores preserving base allocation routing
- governance tilt redistributing a fixed budget
- exact budget conservation under rounding
- comparison against the naive emission-expanding anti-pattern
