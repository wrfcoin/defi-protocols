//! Auto-compounding vaults for WRFCoin yield farms (issue #30).
//!
//! This crate provides a `VaultManager` that owns one or more `ManagedVault`
//! instances, each wrapping a `yield-farming` `FarmManager` pool. Vaults
//! periodically harvest pending rewards and compound them back into the farm
//! position according to a per-vault `VaultConfig`.
//!
//! # Architecture
//!
//! ```text
//! VaultManager
//!   └── ManagedVault (vault_id, VaultConfig)
//!         └── AutoCompoundingVault  (yield-farming)
//!               └── FarmManager pool
//! ```
//!
//! `VaultConfig` is the user-facing configuration surface; it maps 1-to-1 onto
//! `CompoundingStrategy` from the yield-farming crate so that the two crates
//! share a single source of truth for compounding parameters.

pub mod config;
pub mod manager;

pub use config::VaultConfig;
pub use manager::{CompoundResult, ManagedVault, VaultManager};
