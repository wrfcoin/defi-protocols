//! Concentrated liquidity AMM — Uniswap V3-style tick-based pools for WRF pairs.
//!
//! # Overview
//!
//! This module extends `crates/amm/` with a concentrated liquidity system.
//! Unlike the uniform constant-product `LiquidityPool` in `crate::pool`, LPs
//! here deposit liquidity into specific price *ranges* (defined by tick bounds).
//! Only LPs whose range contains the current price earn trading fees.
//!
//! # Architecture
//!
//! ```text
//! ConcentratedLiquidityPool
//!   ├── sqrt_price_current  (SqrtPrice — Q64.64 fixed-point)
//!   ├── current_tick        (i32 — highest tick ≤ current price)
//!   ├── liquidity           (u128 — virtual liquidity active at current price)
//!   ├── tick_map            (HashMap<i32, Tick> — initialized tick data)
//!   ├── tick_bitmap         (TickBitmap — sparse bitmap for tick discovery)
//!   ├── positions           (HashMap<PositionId, ConcentratedPosition>)
//!   └── fee_growth_global   (u128 × 2 — Q64.64 per-unit cumulative fees)
//! ```
//!
//! # Fixed-point choice: Q64.64 vs Q64.96
//!
//! Uniswap V3 uses Q64.96 (160-bit), which requires 256-bit integers (`uint256`
//! in Solidity, `U256` in Rust). This implementation targets `u128` throughout
//! to match `wrfcoin-defi-common::types::Amount`. We use Q64.64 instead:
//!
//! - 64 integer bits → prices up to 2^64 ≈ 1.8 × 10^19 (sufficient for WRF).
//! - 64 fractional bits → sub-unit precision of ~5.4 × 10^-20.
//! - All intermediate products fit in u128 for our supported tick range (±500).
//!
//! See `sqrt_price.rs` for the detailed derivation.
//!
//! # Future: IWrfAMM Conformance
//!
//! This pool is a *standalone* type and does not yet implement the `IWrfAMM`
//! Solidity interface in `contracts/interfaces/IWrfAMM.sol`. A future extension
//! will wrap `ConcentratedLiquidityPool` in a router that adapts it to the
//! `IWrfAMM` high-level swap signature — see defi-protocols issue #41 for the
//! roadmap.
//!
//! # Modules
//!
//! - [`sqrt_price`] — `SqrtPrice` newtype, tick↔price conversion (Q64.64).
//! - [`tick`] — `Tick`, `TickBitmap`, fee-growth-inside helpers, swap step math.
//! - [`position`] — `ConcentratedPosition` with fee accrual and liquidity delta.
//! - [`pool`] — `ConcentratedLiquidityPool` with `mint`/`burn`/`swap`.
//! - [`swap`] — `SwapStep` struct wrapping the low-level step function.

pub mod pool;
pub mod position;
pub mod sqrt_price;
pub mod swap;
pub mod tick;

pub use pool::ConcentratedLiquidityPool;
pub use position::{ConcentratedPosition, PositionId};
pub use sqrt_price::{sqrt_price_to_tick, tick_to_sqrt_price, SqrtPrice, Q64, TICK_MAX, TICK_MIN};
pub use swap::SwapStep;
pub use tick::{
    compute_swap_step, fee_growth_inside, liquidity_to_amounts, Tick, TickBitmap,
};
