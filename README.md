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

### Solidity

```bash
cd contracts
npm install
npx hardhat compile
npx hardhat test
```
