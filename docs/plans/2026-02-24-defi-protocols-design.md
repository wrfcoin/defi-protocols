# DeFi Protocols Design

## Context

Issue #370 describes extracting and building DeFi primitives for WRFCoin. Reference code exists in
`blockchain-core/core/` (~18 files, ~25K lines) but was never wired into any runtime (assessed as
mostly stubs and partial implementations).

## Architecture

Hybrid Rust + Solidity repo. Core DeFi logic in Rust crates, EVM-facing interfaces in Solidity.

```
defi-protocols/
├── Cargo.toml                    # Rust workspace
├── crates/
│   ├── defi-common/              # Shared types & traits
│   ├── lending/                  # Collateralized lending/borrowing
│   ├── yield-farming/            # Yield farming & rewards
│   └── amm/                     # DEX / Automated Market Maker
├── contracts/                    # Solidity EVM interfaces
│   ├── hardhat.config.ts
│   ├── package.json
│   ├── contracts/
│   │   ├── lending/
│   │   ├── yield/
│   │   └── amm/
│   └── test/
└── README.md
```

## Component Placement (per issue #370)

| Component | Location | Rationale |
|-----------|----------|-----------|
| Lending | `defi-protocols/crates/lending/` | Separate state, liquidation logic, oracle feeds |
| Yield farming | `defi-protocols/crates/yield-farming/` | Independent reward mechanics |
| AMM/DEX | `defi-protocols/crates/amm/` | Standalone swap/liquidity logic |
| Liquid staking | `core4/shared-protocol/src/staking/` | Core protocol concern (PoP integration) |
| Account abstraction | `core4/shared-protocol/src/accounts/` | Deep tx validation integration |
| Identity | `core4/shared-protocol/src/identity/` | Oracle registration, reputation |

## Key Traits (defi-common)

```rust
pub trait LendingProtocol {
    fn deposit(&mut self, asset: AssetId, amount: Amount) -> Result<DepositReceipt>;
    fn borrow(&mut self, asset: AssetId, amount: Amount, collateral: AssetId) -> Result<BorrowReceipt>;
    fn repay(&mut self, asset: AssetId, amount: Amount) -> Result<()>;
    fn liquidate(&mut self, borrower: Address, asset: AssetId) -> Result<LiquidationResult>;
}

pub trait AMMPool {
    fn add_liquidity(&mut self, token_a: Amount, token_b: Amount) -> Result<LPTokens>;
    fn swap(&mut self, input: AssetId, amount: Amount) -> Result<SwapResult>;
    fn remove_liquidity(&mut self, lp_tokens: LPTokens) -> Result<(Amount, Amount)>;
}

pub trait YieldFarm {
    fn stake(&mut self, pool_id: PoolId, amount: Amount) -> Result<StakeReceipt>;
    fn harvest(&mut self, pool_id: PoolId) -> Result<Amount>;
    fn unstake(&mut self, pool_id: PoolId, amount: Amount) -> Result<()>;
}
```

## Integration with core4

The `defi-common` crate defines shared types that `core4/shared-protocol` can depend on for liquid
staking work. Lending, AMM, and yield-farming crates are standalone services communicating with
`native-chain` via HTTP/gRPC.

## Decisions

- **Language**: Both Rust (core logic) + Solidity (EVM interface)
- **Scope**: Full DeFi scaffold (lending, yield farming, AMM)
- **Source**: Fresh implementations using blockchain-core as reference only
- **Initial priority**: Lending (highest value per issue #370)
