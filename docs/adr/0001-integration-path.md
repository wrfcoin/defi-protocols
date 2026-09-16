# ADR 0001: How defi-protocols reaches users — integration path

- **Status:** Proposed (2026-09-16) — awaiting owner approval. Once approved, the follow-up wiring
  epic is filed and linked from [#69](https://github.com/wrfcoin/defi-protocols/issues/69).
- **Decision owner:** repo owner. **Drafted by:** the infra#943 build loop.
- **Supersedes:** nothing. **Constrained by:** core4
  [`ADR-001-issue-370-defi-architecture-split.md`](https://github.com/wrfcoin/core4/blob/main/docs/architecture/ADR-001-issue-370-defi-architecture-split.md)
  (Accepted) and core4
  [`2026-02-25-core4-defi-lending-integration-contract-v0.md`](https://github.com/wrfcoin/core4/blob/main/docs/plans/2026-02-25-core4-defi-lending-integration-contract-v0.md).
- **Context record:** [#68](https://github.com/wrfcoin/defi-protocols/issues/68) — DeFi is IN
  SCOPE for the testnet release (owner decision 2026-09-16). Epic:
  [wrfcoin/infra#943](https://github.com/wrfcoin/infra/issues/943).

## Context — what actually exists today (measured 2026-09-16, `main` = `cce68d2`)

| Layer | What is there | Consumers |
|---|---|---|
| Rust workspace | 12 crates, ~20.1K lines, **454 `#[test]`s** (amm 71, lending 65, yield-farming 63, parametric-insurance 49, weather-derivatives 39, defi-common 37, prediction-market 33, revenue-sharing 25, auto-compound-vaults 23, twap-oracle 21, liquid-staking 19, bonding-curve 9). Pure in-memory state machines: **no HTTP framework, no binary target, no persistence, no dependency on `wrfcoin-shared-protocol`** (repo-local `Address`/`Amount`). | **zero** cross-repo consumers |
| Solidity layer (`contracts/`) | 14 `.sol`: 3 core (`LendingPool`, `YieldFarm`, `AMMRouter`), 1 adapter, 6 interfaces, 4 mocks; 12 Hardhat test files. `hardhat.config.ts` configures the compiler only — **no networks**, nothing deployable as-is. Covers **3 of the 12** crates' mechanics. | none |
| core4 | Already owns `shared-protocol/src/staking/liquid_staking.rs` (162 lines) and `shared-protocol/src/accounts/*` (AA, paymaster, recovery, session keys) — exactly the split ADR-001 assigned to core4. `launch_api` has 142 routes in one `routes/mod.rs`; **no** `/defi` or `/staking` routes. Consensus features are gated by the genesis-immutable `FeatureSchedule` (#3092), and `main` is **not rolling-upgradable** — a new consensus module means a fresh genesis cut. | — |
| backend | `shared/core4-route-client.js` is the house proxy pattern (`fetchCore4Json`, `sendNotImplemented`, …); `server/utils/honest-gap.js` (backend#1118) is the 501 marker for unwired capabilities. **No** `/api/defi*` or `/api/staking/liquid*` route exists. | — |
| frontend | `DeFiPortfolio.tsx` (`/defi`, non-MVP) already calls `GET /api/defi/portfolio`; `LiquidStakingDashboard.tsx` (`/staking/liquid`) already calls `/api/staking/liquid/{protocol,user,withdrawals}`. Both are **dead calls** today. | — |
| smart-contracts | A **third**, unrelated DeFi contract set (`CarbonCreditAMM`, `YieldFarmingManager`, `MultiAssetStaking`, `StakeDelegation`, …) with its own interfaces, and — per smart-contracts#234 — no real deployment anywhere. | none |

Two facts drive the decision: the value is overwhelmingly in the **Rust crates** (20K lines, 454
tests, 12 mechanisms) and those crates are **pure libraries with no runtime**. Whatever path we
pick has to give them a runtime and a settlement story without throwing the tests away.

## Options

### (a) Native — merge the crates into core4 `shared-protocol` / `native-chain`

Swaps, deposits, borrows become consensus state transitions; reachable via new `launch_api`
routes and a `wrfcoin defi` CLI group.

- **Cost.** Every crate becomes consensus code: deterministic execution in the block applier,
  state-root inclusion, replay, persistence migrations (#3091), and a `FeatureSchedule` entry —
  which is genesis content, so it needs a **fresh genesis cut**. The AA cut is already planned at
  the current `main` head (`72a993552`); DeFi would either miss that cut or delay it. The closest
  analog for *one* consensus-adjacent feature, paymaster (core4#3146), was +1,607/-40 across 16
  files; twelve mechanisms with balance-moving semantics is an order of magnitude more, plus a
  determinism review of every decimal path. It also contradicts ADR-001, which deliberately kept
  lending/AMM/yield **out** of core4 and put only liquid staking / AA / identity there.
- **Benefit.** Non-custodial, trustless, the "most consistent with the platform" outcome; no new
  service to operate.
- **Verdict.** Right end-state for *some* primitives (liquid staking is already there), wrong
  first step for the testnet release: highest cost, couples DeFi to the genesis-cut calendar, and
  overrides an accepted ADR.

### (b) Standalone service — a Rust HTTP daemon from this repo, proxied by backend

Add a `defi-node` binary (axum) that hosts the crates' state machines, consumed by backend via a
new `server/routes/defi-routes.ts` at `/api/defi`, which the frontend already calls.

- **Cost.** One new crate + Dockerfile + compose service (the exact shape storage-providers#65 →
  infra#944 → backend#1117 uses), one backend proxy router, one infra compose entry. Zero changes
  to the 12 crates or their 454 tests; zero frontend change for the first slice; no genesis cut;
  no second chain.
- **The honest limitation.** WRF balances live on core4. A service cannot move them, so its ledger
  is **off-chain and custodial**: users deposit/withdraw with signed core4 transactions to a
  service-controlled address, and the service's pools/positions are its own bookkeeping until a
  native settlement path exists. That is acceptable for a testnet *if every response says so*
  (`simulated`/`custodial: true`, the honest-gap convention from backend#1118) and if the first
  slice is read/quote-only so no funds are at risk before the settlement story is designed.
- **Benefit.** Fastest path to a real, testable surface; uses the house pattern; every crate's
  tests keep counting; the operational cost is one more container on the N5 (local-only deploy).
- **Verdict.** **Recommended** for the testnet phase, explicitly labelled off-chain/custodial, with
  the settlement design as its own follow-up story.

### (c) EVM-deployed — deploy the Solidity layer to a real EVM testnet, frontend via wallet-connect

- **Cost.** `contracts/hardhat.config.ts` has no networks, so this starts from zero deployment
  hygiene (the same gap smart-contracts#234 documents). WRF is a native-chain asset: an EVM DeFi
  layer needs a bridged/wrapped WRF, i.e. the core4↔EVM bridge live end-to-end first. The
  frontend has no wallet-connect flow. And the platform would run **two chains**. The Solidity
  layer covers only **3 of 12** mechanisms, so 9 crates (insurance, derivatives, prediction
  market, vaults, revenue sharing, TWAP, …) would be unreachable, and the 454 Rust tests would be
  bypassed in favour of 12 Hardhat files. There would then be **three** overlapping DeFi contract
  sets (this repo's, smart-contracts', and whatever the bridge needs) to reconcile.
- **Benefit.** Uses the contracts that already exist; composable with the wider EVM ecosystem;
  would resolve the "no real deployment" finding for this repo's contracts at the same time.
- **Verdict.** Rejected for this phase. Revisit once the bridge is live and wrapped WRF exists;
  ADR-001 already rejected "full Solidity implementation" for core logic.

## Decision

**Adopt (b): defi-protocols ships a standalone Rust HTTP service (`defi-node`) that backend proxies
at `/api/defi`, labelled off-chain/custodial in every response until a settlement path lands.**

Two carve-outs keep this consistent with ADR-001:

1. **Liquid staking stays native.** core4 already owns `shared-protocol/src/staking/liquid_staking.rs`.
   This repo's `crates/liquid-staking` (903 lines, 19 tests, stWRF rebasing shares) is the richer
   reference; reconciling the two (core4 = consensus owner, this crate = reference/spec) is a
   separate core4 story, **not** part of `defi-node`. `/api/staking/liquid/*` is therefore served
   by backend proxying core4, not by `defi-node`.
2. **The Solidity layer is not deleted.** It stays as the EVM adapter per ADR-001, but no story in
   the follow-up epic targets it until (c)'s prerequisites exist.

## The first wiring story (concrete, so the follow-up epic is not abstract)

**Story A — `defi-node` v0 (this repo).** New crate `crates/defi-node`, bin `defi-node`, `axum` +
`tokio`, bind from `DEFI_NODE_BIND` (proposed `0.0.0.0:7100`; verify against backend
`config/port-registry.json` and infra compose before use — 8080 is contended per infra#421).
Routes:

| Route | Backed by | Response contract |
|---|---|---|
| `GET /health` | — | standard probe |
| `GET /api/defi/pools` | `amm::LiquidityPool` seeded from a checked-in TOML fixture | `{ simulated: true, custodial: true, pools: [...] }` |
| `POST /api/defi/pools/{id}/quote` | `amm::SwapEngine` (pure, no state change) | quote + the same flags |
| `GET /api/defi/portfolio/{address}` | nothing yet | honest-gap `501 { unavailable: true, reason: "no ledger until the settlement story" }` |

Plus `Dockerfile` (multi-stage, distroless/alpine runtime), a `tests/http_contract.rs` that boots
the bin on an ephemeral port and exercises all four routes, and a README section documenting the
contract. Acceptance: `cargo test -p defi-node` green; image builds; every non-501 body carries
`simulated: true`.

**Story B — backend proxy.** `server/routes/defi-routes.ts` mounted with
`safeMount('/api/defi', …)` in `enhanced_unified_api_optimized.ts`, forwarding to `DEFI_NODE_URL`
via the `core4-route-client.js` pattern; when the env is unset, answer with
`server/utils/honest-gap.js` `sendNotWired()` (never a mock). Tests mirror
`__tests__/honest-gap-servers.test.ts`.

**Story C — infra.** `wrfcoin-defi` service in the local testnet compose (local-only deploy to the
N5, never a registry push), healthcheck on `/health`, `DEFI_NODE_URL` injected into the backend
service. Smoke: `curl /api/defi/pools` through backend returns `simulated: true`.

**Story D — settlement design (blocking any balance-moving route).** Decide the custody model:
deposit/withdraw via core4 `/transactions/submit` to a service address, ledger persistence (the
service's own store vs backend's Postgres), and the reconciliation job. This is a design story,
not code, and it gates every route beyond quote/read.

Frontend needs **no** change for stories A–C (`DeFiPortfolio` already calls `/api/defi/portfolio`);
rendering the `simulated` flag is a later frontend story.

## Consequences

- **Positive:** real surface within one sprint; crates and tests reused intact; no genesis-cut
  coupling; one operational pattern shared with storage-providers.
- **Negative:** custodial/off-chain until Story D and its successor land; must be said in every
  response and in the README, or it becomes exactly the "armed, not firing" trap this epic exists
  to remove. One more container to run.
- **Neutral:** the Solidity layer and smart-contracts' own DeFi contracts remain unreconciled;
  that reconciliation is a prerequisite of (c), not of (b).

## What this ADR does not do

It does not file the follow-up epic (that waits for approval of this decision) and it does not
size Story D. It does not change `WRFCOIN_API_URL` semantics or any CLI surface; a `wrfcoin defi`
command group would arrive with Story B's routes, using the per-family `ServiceTarget` pattern from
core4#3204.
