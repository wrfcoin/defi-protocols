# Compute Credit Interoperability

`crates/defi-common/src/compute_credit.rs` defines the receipt-backed accounting layer for fee-less economics.

Expected flow:

1. The core runtime validates useful work and emits a unique compute receipt.
2. DeFi services mint credits exactly once from that receipt via `mint_from_receipt`.
3. Applications reserve and spend credits deterministically through `reserve` and `spend_reserved`.
4. Transfers move credits into child lots so lineage stays reviewable.
5. Slashing or expiration burns remaining supply and updates the per-lot lifecycle state.

Interoperability expectations:

- `ComputeReceiptId` must be unique across the runtime domain to guarantee replay protection.
- `beneficiary` is the account that receives spendable credits after proof validation.
- `contributor` records the originator of useful work and is used to block circular routing back to the source.
- Gas sponsorship / paymaster flows should only consume credits that have been explicitly reserved for settlement.
- Runtime adapters should set `expires_at` conservatively so old receipts cannot be reintroduced after policy changes.
