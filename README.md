# WRFCoin DeFi Protocols

Reference Rust libraries and Solidity interfaces for planned WRFCoin DeFi protocols.

**These mirror planned protocol features and are NOT part of the running testnet.** The opt-in simulator is read/quote-only; it has no custody, settlement, or chain writes.

## Public testnet

Test status (2026-10-19): `cargo test` passed at `0e3fc1f54ab1` on Linux (462 passed, 0 ignored; default features; local tests, not deployment evidence).

Public testnet launch: **2026-10-22**. Testnet tokens have no monetary value, and the network may be reset.

- [WRFCoin website](https://wrfcoin.com)
- [Public testnet quickstart](https://github.com/wrfcoin/docs/blob/main/testnet/quickstart.md)
- [Help and bug reports](https://github.com/wrfcoin/testnet-support)
- [Security reporting](SECURITY.md)

## Today and planned work

| Status | Scope |
| --- | --- |
| Available in this repository today | Rust protocol libraries and an opt-in simulator with responses labelled `simulated:true`. |
| Planned / not available here | Live settlement and custody are not implemented. [ADR 0002](docs/adr/0002-settlement-and-custody.md) records the read/quote-only decision. |

## Build and use

See [CONTRIBUTING.md](CONTRIBUTING.md#build-and-test) for prerequisites and build/test commands. CI results apply only to the named commit and executed checks; they do not prove deployment.

Licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.

> ## Testnet status: simulator only, not live
>
> The opt-in `defi-node` v0 surface is read/quote-only. It does not hold custody, move balances,
> or touch chain state. Every response is explicitly labelled `simulated:true`; settlement is
> unavailable under the accepted read/quote-only ADR 0002. See [ADR 0001](docs/adr/0001-integration-path.md).

DeFi protocol implementations for the WRFCoin platform. Hybrid Rust + Solidity architecture with
core logic in Rust crates and EVM-facing interfaces in Solidity.


## Structure

```
crates/
├── defi-common/      # Shared types, traits, errors
├── lending/          # Collateralized lending/borrowing
├── yield-farming/    # Yield farming & reward distribution
├── bonding-curve/    # Weather data subscription pricing curves
└── amm/              # DEX / Automated Market Maker

contracts/            # Solidity EVM interfaces
├── contracts/
│   ├── lending/      # LendingPool.sol
│   ├── yield/        # YieldFarm.sol
│   ├── amm/          # AMMRouter.sol
│   └── interfaces/   # Protocol interfaces
├── hardhat.config.ts
└── package.json
```

## Development

### Rust

```bash
cargo check
cargo test
cargo build
```

### Incentive Backtest

```bash
python3 scripts/simulate_utility_incentives.py
```

### Governance Guardrails

All DeFi feature proposals must satisfy mission-alignment guardrails before merge/release.

```bash
python3 scripts/check_mission_guardrails.py --repo-root .
python3 -m unittest scripts/test_check_mission_guardrails.py
```

Guardrail sources:
- `docs/governance/mission-guardrails.toml`
- `docs/governance/mission-guardrail-status.md`
- `docs/governance/module-guardrail-status.toml`
- `docs/governance/module-deprecation-recovery.md`
- `docs/proposals/defi-feature-proposal-template.md`

Deprecation gate semantics:
- `active` => no required guardrail failures
- `deprecated` => guardrail failures present; release is blocked
- `remediation` => failures cleared and monitored before reactivation

### Privacy Risk Simulation

```bash
python3 -B scripts/score_metadata_leakage_risk.py
```

### Privacy Design Docs (Issue #8)

- `docs/privacy/2026-03-04-defi-phase2-privacy-reward-flow.md`
- `docs/privacy/2026-03-04-defi-phase2-data-minimization-requirements.md`
- `docs/privacy/2026-03-04-defi-phase2-threat-model.md`
- `docs/simulations/2026-03-04-metadata-leakage-risk-simulation.md`

### Verified-Utility Escrow (Issue #9)

- `docs/incentives/2026-03-04-verified-utility-escrow.md`

### Solidity

```bash
cd contracts
npm install
npx hardhat compile
npx hardhat test
```
