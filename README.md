# WRFCoin DeFi Protocols

DeFi protocol implementations for the WRFCoin platform. Hybrid Rust + Solidity architecture with
core logic in Rust crates and EVM-facing interfaces in Solidity.

Related: [issue #370](https://github.com/wrfcoin/core4/issues/370)

## Structure

```
crates/
├── defi-common/      # Shared types, traits, errors
├── lending/          # Collateralized lending/borrowing
├── yield-farming/    # Yield farming & reward distribution
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

### Governance Guardrails

All DeFi feature proposals must satisfy mission-alignment guardrails before merge/release.

```bash
python3 scripts/check_mission_guardrails.py --repo-root .
python3 -m unittest scripts/test_check_mission_guardrails.py
```

Guardrail sources:
- `docs/governance/mission-guardrails.toml`
- `docs/governance/mission-guardrail-status.md`
- `docs/proposals/defi-feature-proposal-template.md`

### Solidity

```bash
cd contracts
npm install
npx hardhat compile
npx hardhat test
```
