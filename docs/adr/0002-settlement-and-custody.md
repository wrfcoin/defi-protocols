# ADR 0002: DeFi settlement and custody model

- **Status:** **Accepted 2026-09-21: option 1 (read/quote-only for the whole testnet phase).**
  The owner decided this in
  [defi-protocols#74, DECISION comment of 2026-09-21](https://github.com/wrfcoin/defi-protocols/issues/74#issuecomment-5764214607).
  To overturn it, reopen #74 with a comment.
- **Decision owner:** repo owner. **Drafted by:** the close-list lane, 2026-09-27, recording the ruling above.
- **Supersedes:** nothing. **Constrained by:** [ADR 0001](0001-integration-path.md) (Accepted
  2026-09-18, simulator only), whose "Story D" this record answers, and the Wave 0 no-custody
  ruling on [#72](https://github.com/wrfcoin/defi-protocols/issues/72).
- **Epic:** [wrfcoin/infra#966](https://github.com/wrfcoin/infra/issues/966).

> **This record authorizes no implementation.** It adds no route, crate, service key, ledger or
> deployment. Its only effect is the merge rule in the section "Rule while this status holds".

## Context

WRF balances live on core4. `defi-node` (ADR 0001 option b, shipped in #75) is a side service
that has no ledger and holds no keys. Every body it returns carries `simulated:true`,
`custodial:false` and `settlement:"none"` (`crates/defi-node/src/envelope.rs`). Any deposit,
swap, borrow or withdraw route shipped before this question is settled would do one of two
things:
- fabricate balances, or
- take custody of testnet funds with no reconciliation story.

That is the "manufactured success" class that readiness rounds IV–V removed elsewhere.

Measured facts this record depends on (defi-protocols `main` = `dcdb390`, core4 `main` = `9ef92a235`):

| Fact | Where |
|---|---|
| DeFi amounts are `Amount(pub u128)` in base units | `crates/defi-common/src/types.rs` (`pub struct Amount(pub u128)`) |
| Both repos use the same base unit: `WRF_UNITS_PER_COIN = 100_000_000` (1 WRF = 10^8 base units) | defi-protocols `crates/defi-common/src/types.rs`, core4 `shared-protocol/src/common/constants.rs:39` (`u64`) |
| core4 transfer amounts are `u64` | core4 `shared-protocol/src/blockchain/transaction.rs:203` (`pub amount: u64`) |
| No crate in this repo has a persistence layer | `crates/*` @ `dcdb390` |
| **Width** conversion already exists and is checked: `impl From<u64> for Amount` (lossless widening) and `impl TryFrom<Amount> for u64` (returns `TryFromIntError` above `u64::MAX`), with tests `test_amount_from_u64`, `test_amount_try_into_u64` and `test_amount_try_into_u64_overflow`. These were added in `69ad7af` (the #34 fix). | `crates/defi-common/src/types.rs:98-111`, tests `:567-589` |
| What does **not** exist is a **settlement integration**. No code converts a DeFi position into a core4 transaction or back. The existing width conversions are **asset-unaware**: they take a bare `Amount` with no `AssetId`. No crate outside `defi-common` calls them. | `git grep` over `crates/` at `dcdb390` |

## Options

Each option is costed on the same axes: key custody, ledger store, reconciliation,
`u128↔u64` conversion, deposit replay/idempotency, response flags, and failure/refund path.

### Option 1: stay read/quote-only for the whole testnet phase **(chosen)**

- **Key custody:** none. No service key exists, so there is nothing to hold, rotate or lose.
- **Ledger store:** none. Portfolio reads keep answering `501 { unavailable: true }`.
- **Reconciliation:** nothing to reconcile, because the service holds no liabilities.
- **`u128↔u64`:** there are no conversions on a settlement path. Quotes are computed and returned
  in `u128` base units, and never become a core4 transaction.
- **Replay/idempotency:** there are no deposits.
- **Flags:** unchanged. `simulated:true`, `custodial:false`, `settlement:"none"`.
- **Failure/refund:** nothing can be lost, so nothing needs refunding.
- **Cost:** zero. **Risk:** zero funds at risk. **Limitation:** DeFi is visible and quotable on the
  testnet but never settles. This keeps the 2026-09-16 "DeFi in testnet scope" decision
  ([#68](https://github.com/wrfcoin/defi-protocols/issues/68)) honest.

### Option 2: custodial service ledger (not chosen)

A user sends a signed core4 transfer to a service address through core4
`POST /transactions/submit`. The service credits an off-chain ledger. Withdrawals are transfers
the service signs.

- **Key custody:** the service address key would need an HSM (SoftHSM at minimum, as the
  producer key does without a YubiKey), an owner key ceremony, and a rotation and loss runbook.
  Whoever holds it can move every deposited WRF.
- **Ledger store:** either service-local (redb/sqlite), which is simple but adds a new backup
  surface, or backend Postgres, which reuses the migrations and backups but couples the two
  services. Either way, the ledger is the source of truth for user positions.
- **Reconciliation:** a job that compares the ledger with core4 at finalized heights. The halt
  rule is below.
- **`u128↔u64`:** the conversion rule below applies on every deposit credit and withdrawal
  debit.
- **Replay/idempotency:** a deposit is keyed by its core4 transaction hash. It is credited
  exactly once, only after finality, and a second credit for the same hash is rejected, not
  ignored.
- **Flags:** `simulated:false`, `custodial:true`, `settlement:"service-ledger"`, only for
  routes that pass reconciliation.
- **Failure/refund:** an unmatched or malformed deposit is refunded by a service-signed transfer
  to the sender, in full, and the refund is recorded in the ledger. If the refund itself
  fails, the balance-moving routes halt.

### Option 3: native settlement in core4 (not chosen)

The DeFi state transitions become consensus code in core4.

- **Key custody:** none beyond users' own keys. It is non-custodial.
- **Ledger store:** core4 state, which is covered by the state root.
- **Reconciliation:** consensus itself.
- **`u128↔u64`:** the conversion rule below applies at the crate boundary, inside consensus.
- **Replay/idempotency:** handled by core4 nonces.
- **Flags:** `simulated:false`, `custodial:false`, `settlement:"native"`.
- **Failure/refund:** transactions revert atomically.
- **Cost:** highest. It needs a `FeatureSchedule` entry, and therefore either a genesis cut or
  the upgrade-in-place path of [core4#3090](https://github.com/wrfcoin/core4/issues/3090) /
  core4#3277. It also needs a determinism review of every decimal path, and it overrides the
  core4 ADR-001 split.

## Rules that any future settlement option must meet (specified so they can be tested)

These rules do not apply while option 1 holds, because nothing converts or reconciles. They are
fixed now so that a later option 2 or 3 story cannot redefine them.

### `u128 ↔ u64` conversion rule

1. **Units:** both sides are WRF base units (10^8 per WRF). There is no rescaling, in either direction.
2. **Widening (core4 → DeFi):** reuse the existing `impl From<u64> for Amount`
   (`defi-common/src/types.rs:98`). This is lossless and cannot fail.
3. **Narrowing (DeFi → core4):** reuse the existing checked `impl TryFrom<Amount> for u64`
   (`types.rs:104-111`). If the value exceeds `u64::MAX`, the operation is **rejected** with the
   typed error, and no transaction is built. Saturating, truncating, wrapping and splitting are
   all forbidden, so the checked impl must stay the only narrowing path.
4. **Non-WRF assets (the missing piece):** the existing impls are width-only and asset-unaware.
   A settlement boundary must add an **asset-aware** wrapper that takes the `AssetId` along with
   the `Amount`, rejects anything that is not native WRF, and only then calls the checked width
   conversion. An `Amount` whose asset is not native WRF is never converted to a core4 amount.
5. **Test vectors** (required by any implementing PR, on the asset-aware wrapper. The existing
   width tests at `types.rs:567-589` already cover `u64::MAX` and `u64::MAX + 1` for the width
   layer):
   - `0 → 0`
   - `u64::MAX` round-trips exactly
   - `u64::MAX as u128 + 1` gives an error
   - `u128::MAX` gives an error
   - a non-WRF asset gives an error
   - a mutation that saturates instead of erroring must turn the suite RED.

### Reconciliation halt condition (option 2)

Let `L` be the sum of all ledger liabilities in base units (`u128`). Let `B` be the core4 balance
of the service address at the latest **finalized** height `h`, widened to `u128`.

- **Invariant:** `L ≤ B` at every reconciliation, with **zero tolerance**. `B − L` must equal the
  recorded fee reserve, and any other surplus is itself reported as a mismatch.
- **Halt:** every balance-moving route (credit, swap-that-debits, withdraw, refund) returns
  `503 { halted: true, reason }`, and read and quote routes keep serving, when any of the
  following holds:
  1. `L > B` at height `h`.
  2. `B − L` differs from the recorded fee reserve.
  3. The job fails to read `B` at a finalized height for **3 consecutive** intervals.
  4. `h` does not advance for 3 consecutive intervals, which means the job would be reconciling
     against a stale height.
- **Resume:** only after an operator reconciles by hand and records a signed note in the
  ledger. The halt is never cleared automatically.
- **Tests (required by any implementing PR):**
  - one fixture per halt trigger, each asserting 503 on a balance-moving route and 200 on a
    quote route
  - a mutation that removes the check must go RED.

## Decision

**Option 1.** DeFi stays read/quote-only for the whole testnet phase. No DeFi or backend route
moves WRF. Options 2 and 3 are not chosen for this phase. Nothing here rules them out for a
later phase, but choosing either one requires a new, dated decision in this record.

### Rule while this status holds

**While this record's status is anything other than "Accepted" with option 2 or option 3, no
balance-moving route may merge in `wrfcoin/defi-protocols` or `wrfcoin/backend`.**

A route counts as balance-moving if it does any of the following:
- credits or debits a position
- accepts a deposit
- builds, signs or submits a core4 transaction
- returns a body with `simulated:false` or `custodial:true`

Reviewers should cite this section when they block such a PR. `defi-node` and backend's
`/api/defi` proxy stay read/quote-only, with `simulated:true`, `custodial:false` and
`settlement:"none"`.

### Decision log

- **2026-09-21:** option 1 Accepted by the owner
  ([#74 DECISION comment](https://github.com/wrfcoin/defi-protocols/issues/74#issuecomment-5764214607)).
  This keeps the 2026-09-16 in-scope decision ([#68](https://github.com/wrfcoin/defi-protocols/issues/68))
  and the 2026-09-18 no-custody ruling ([#72](https://github.com/wrfcoin/defi-protocols/issues/72)).
- **2026-09-27:** this file was written to record that ruling. The decision itself did not change.

## What each option unblocks

| Option | Routes it permits |
|---|---|
| **1 (Accepted)** | `GET /health`, `GET /api/defi/pools`, `POST /api/defi/pools/{id}/quote` (pure, no state change), and `GET /api/defi/portfolio/{address}` as an honest `501`. Backend `/api/defi` proxy forwarding those only, with an honest 501 when unconfigured (backend#1179). Opt-in local compose overlay (infra#980). Nothing that moves a balance. |
| 2 (not chosen) | In addition: deposit credit (keyed by core4 transaction hash, after finality), ledger-backed portfolio reads, service-signed withdraw and refund, and swap/borrow against ledger positions. Every one of these is gated by the reconciliation halt and the conversion rule. |
| 3 (not chosen) | In addition: `launch_api` DeFi transaction types and a `wrfcoin defi` CLI group, behind a scheduled consensus feature. |

## Consequences

- **Positive:** no funds at risk, no key ceremony, no ledger to back up, and no coupling to the
  genesis-cut calendar. The DeFi surface stays honest and labelled.
- **Negative:** no DeFi position is real during the testnet phase. Users can quote but never
  settle.
- **Neutral:** the conversion and reconciliation rules above are fixed now, so any later
  settlement story starts from a testable contract instead of a design debate.
