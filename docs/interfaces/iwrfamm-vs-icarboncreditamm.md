# IWrfAMM vs ICarbonCreditAMM — Interface Split Rationale

## Overview

WRFCoin has two AMM interfaces that serve distinct asset classes and must NOT be conflated:

| Interface | Repo | Implementation | Asset Class |
|-----------|------|----------------|-------------|
| `IWrfAMM` | `wrfcoin/defi-protocols` | `AMMRouter` (Rust) | General WRF/token pairs, governance tokens, yield-bearing assets |
| `ICarbonCreditAMM` | `wrfcoin/smart-contracts` | `CarbonCreditAMM` (Solidity) | Carbon credit ERC-20 pairs |

## Why they do NOT share an interface

The swap signatures are fundamentally incompatible:

**IWrfAMM.swap** (high-level router, slippage guard built in):
```solidity
function swap(
    address tokenIn,
    address tokenOut,
    uint256 amountIn,
    uint256 amountOutMin
) external returns (uint256 amountOut);
```

**CarbonCreditAMM.swap** (Uniswap-v2 pair, transfer-then-swap pattern):
```solidity
function swap(
    uint256 amount0Out,
    uint256 amount1Out,
    address to,
    bytes calldata data
) external;
```

`IWrfAMM.swap` accepts token addresses and an input amount and handles transfers internally.
`CarbonCreditAMM.swap` expects the input tokens to already be in the contract before the call,
takes *output* amounts rather than an input amount, and supports flash-swap callbacks via `data`.
These differences are irreconcilable in a shared interface without hiding behavior that callers
must understand.

## Layered-adapter pattern

`AMMRouter` (Rust, `crates/amm/src/router.rs`) routes through `CarbonCreditAMM` pairs via a
thin adapter that bridges the two calling conventions:

```
Caller
  │
  ▼
AMMRouter  (implements IWrfAMM — high-level routing, fee-tier selection, multi-hop)
  │  for carbon-credit hops
  ▼
CarbonCreditAMMAdapter  (converts IWrfAMM call → transfer-then-swap)
  │  1. transfer amountIn to CarbonCreditAMM
  │  2. call sync() to update reserves
  │  3. compute amount0Out / amount1Out via getAmountOut()
  │  4. call CarbonCreditAMM.swap(amount0Out, amount1Out, recipient, "")
  ▼
CarbonCreditAMM  (Uniswap-v2 pair — carbon assets only, ERC-20 LP tokens, flash loans)
```

## Fee model alignment

Both interfaces use basis-point fees with `FEE_DENOMINATOR_BPS = 10_000`:

| Contract | Fee parameter | Default | Tiers |
|----------|--------------|---------|-------|
| `AMMRouter` / `IWrfAMM` | `feeBps` per pair | 30 bps (0.3%) | 1 / 5 / 30 / 100 bps |
| `CarbonCreditAMM` | `protocolFeeNumerator / protocolFeeDenominator` | 30 / 10000 | configurable |

See `crates/defi-common/src/types.rs` for the shared fee constants.

## When to use which

- **General token trading, governance, yield** → `IWrfAMM` / `AMMRouter`
- **Carbon credit trading** → `ICarbonCreditAMM` / `CarbonCreditAMM`
- **Routing across both** → `AMMRouter` with `CarbonCreditAMMAdapter`
- **Adding a new asset type** → never force a carbon credit pair into `IWrfAMM` or vice versa

## Cross-references

- `IWrfAMM` full NatSpec: `contracts/contracts/interfaces/IWrfAMM.sol` in `wrfcoin/defi-protocols`
- `CarbonCreditAMM` implementation: `CarbonCreditAMM.sol` in `wrfcoin/smart-contracts`
- Router implementation: `crates/amm/src/router.rs`
- Fee constants: `crates/defi-common/src/types.rs`
