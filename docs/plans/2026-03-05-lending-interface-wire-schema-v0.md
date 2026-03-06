# Lending Interface Wire Schema V0

This repository now defines a local `lending-interface-v0` wire schema in `crates/lending/src/wire.rs`.

Scope:

- `LendingAccountId` deterministic conversion from repo-local `Address`
- decimal-string `principal`, `collateral_value`, and `oracle_price` fields
- bounded risk basis-point wrappers for max LTV, liquidation threshold, and liquidation penalty
- schema-level validation for versioning and risk relationships

Reference:

- Changelog / issue note references the external integration contract doc path:
  `core4/docs/plans/2026-02-25-core4-defi-lending-integration-contract-v0.md`
