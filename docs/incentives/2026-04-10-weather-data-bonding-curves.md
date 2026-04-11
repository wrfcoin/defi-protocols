# Weather Data Bonding Curves (defi-protocols#45)

Date: 2026-04-10

## Goal

Price weather-data subscriptions as a primary-market primitive instead of treating them like a
normal AMM pair. Buyers mint subscription tokens directly from a deterministic curve, and each
purchase splits value between reserve backing and contributor funding.

## Rust-First Deliverable

This PR centers on a new Rust crate:

- `crates/bonding-curve/`

The crate models:

- sigmoid pricing as the primary launch mode
- exponential pricing as an alternate option for steeper launch profiles
- linear and polynomial curves as natural extensions
- buy / sell quote and execution paths
- reserve-ratio split bookkeeping
- capped-growth guardrails inspired by the `#47` reward-stacking hardening

## Reserve Split

Every subscription purchase is split into:

- `reserve_added`: stays inside the market to fund redemptions
- `contributor_funding`: non-reserve revenue that can later be routed to the revenue-sharing flow from `#44`

That means the market can continuously fund weather-data producers without pretending those flows
are protocol emissions.

## Guardrails

To avoid a stacked-emission-style runaway path, the market applies two hard limits:

1. `max_outstanding_supply`
   Caps the total number of outstanding subscription tokens for a region/tier market.

2. `max_price_multiplier_bps`
   Caps the marginal price relative to the launch price, so an aggressive curve cannot grow
   without bound.

These guardrails mirror the shape of `#47`:

- cap the surface area
- cap the multiplier

without reusing the yield-farm emission mechanism itself.

The sigmoid curve keeps the launch price low, then bends toward a ceiling as demand increases.
Exponential remains available for markets that want a faster ramp, but it is no longer the
only supported launch shape.

## Scope Choice

The issue mentions a Solidity companion, but this draft intentionally keeps the launch-prep
deliverable Rust-first so we can land pricing math, reserve accounting, and safety tests without
blocking on a broader EVM integration surface.
