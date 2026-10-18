# WRFCoin DeFi Protocols

## Public testnet

Public testnet launch: **2026-10-22**. Testnet tokens have no monetary value.
Visit [WRFCoin](https://wrfcoin.com), read the [documentation](https://github.com/wrfcoin/docs),
and ask for help in [testnet-support](https://github.com/wrfcoin/testnet-support).

> ## Testnet status: simulator only, not live
>
> The opt-in `defi-node` v0 surface is read/quote-only. It does not hold custody, move balances,
> or touch chain state. Every response is explicitly labelled `simulated:true`; settlement is
> blocked on a separately accepted ADR 0002. See [ADR 0001](docs/adr/0001-integration-path.md).

DeFi protocol implementations for the WRFCoin platform. Hybrid Rust + Solidity architecture with
core logic in Rust crates and EVM-facing interfaces in Solidity.

Related: [issue #370](https://github.com/wrfcoin/core4/issues/370) (original exploration ticket)

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
