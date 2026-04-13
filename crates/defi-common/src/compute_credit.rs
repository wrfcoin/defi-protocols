use crate::error::{DefiError, DefiResult};
use crate::types::{Address, Amount, Timestamp};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ComputeReceiptId(pub String);

impl ComputeReceiptId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ComputeCreditStatus {
    Minted,
    PartiallyReserved,
    FullyReserved,
    PartiallySpent,
    Spent,
    Slashed,
    Expired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComputeReceipt {
    pub id: ComputeReceiptId,
    pub contributor: Address,
    pub beneficiary: Address,
    pub credits: Amount,
    pub compute_units: u64,
    pub issued_at: Timestamp,
    pub expires_at: Timestamp,
    pub runtime_nonce: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComputeCreditLot {
    pub receipt: ComputeReceipt,
    pub owner: Address,
    pub available: Amount,
    pub reserved: Amount,
    pub spent: Amount,
    pub slashed: Amount,
    pub status: ComputeCreditStatus,
    pub lineage_root: ComputeReceiptId,
    pub transfer_generation: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComputeCreditLedger {
    lots: HashMap<ComputeReceiptId, ComputeCreditLot>,
    balances: HashMap<Address, Amount>,
    reserved_balances: HashMap<Address, Amount>,
    child_counters: HashMap<ComputeReceiptId, u32>,
    total_supply: Amount,
}

impl Default for ComputeCreditLedger {
    fn default() -> Self {
        Self::new()
    }
}

impl ComputeCreditLedger {
    pub fn new() -> Self {
        Self {
            lots: HashMap::new(),
            balances: HashMap::new(),
            reserved_balances: HashMap::new(),
            child_counters: HashMap::new(),
            total_supply: Amount::ZERO,
        }
    }

    pub fn total_supply(&self) -> Amount {
        self.total_supply
    }

    pub fn balance_of(&self, owner: Address) -> Amount {
        self.balances.get(&owner).copied().unwrap_or(Amount::ZERO)
    }

    pub fn reserved_balance_of(&self, owner: Address) -> Amount {
        self.reserved_balances
            .get(&owner)
            .copied()
            .unwrap_or(Amount::ZERO)
    }

    pub fn lot(&self, receipt_id: &ComputeReceiptId) -> Option<&ComputeCreditLot> {
        self.lots.get(receipt_id)
    }

    pub fn mint_from_receipt(&mut self, receipt: ComputeReceipt) -> DefiResult<()> {
        self.validate_receipt(&receipt)?;
        if self.lots.contains_key(&receipt.id) {
            return Err(DefiError::DuplicateComputeReceipt {
                receipt_id: receipt.id.0.clone(),
            });
        }

        self.total_supply = self
            .total_supply
            .checked_add(receipt.credits)
            .ok_or_else(|| DefiError::Overflow {
                operation: "compute-credit total supply mint".into(),
            })?;

        self.increment_balance(receipt.beneficiary, receipt.credits)?;

        self.lots.insert(
            receipt.id.clone(),
            ComputeCreditLot {
                owner: receipt.beneficiary,
                available: receipt.credits,
                reserved: Amount::ZERO,
                spent: Amount::ZERO,
                slashed: Amount::ZERO,
                status: ComputeCreditStatus::Minted,
                lineage_root: receipt.id.clone(),
                transfer_generation: 0,
                receipt,
            },
        );

        Ok(())
    }

    pub fn transfer(
        &mut self,
        receipt_id: &ComputeReceiptId,
        from: Address,
        to: Address,
        amount: Amount,
    ) -> DefiResult<ComputeReceiptId> {
        if amount.is_zero() {
            return Err(DefiError::InvalidAmount {
                reason: "transfer amount must be greater than zero".into(),
            });
        }
        if from == to {
            return Err(DefiError::ComputeCreditAbuse {
                reason: "self transfer / wash cycling is not allowed".into(),
            });
        }

        let (lineage_root, contributor, parent_generation) = {
            let lot =
                self.lots
                    .get_mut(receipt_id)
                    .ok_or_else(|| DefiError::InvalidComputeReceipt {
                        reason: format!("unknown receipt lot {}", receipt_id.as_str()),
                    })?;

            if lot.owner != from {
                return Err(DefiError::Unauthorized {
                    reason: "transfer source does not own receipt lot".into(),
                });
            }
            if to == lot.receipt.contributor {
                return Err(DefiError::ComputeCreditAbuse {
                    reason: "circular sponsorship back to the contributor is not allowed".into(),
                });
            }
            if lot.available < amount {
                return Err(DefiError::InsufficientBalance {
                    required: amount,
                    available: lot.available,
                });
            }

            lot.available =
                lot.available
                    .checked_sub(amount)
                    .ok_or_else(|| DefiError::Overflow {
                        operation: "compute-credit transfer source lot".into(),
                    })?;
            lot.status = status_for(lot.available, lot.reserved, lot.spent, lot.slashed);
            (
                lot.lineage_root.clone(),
                lot.receipt.contributor,
                lot.transfer_generation,
            )
        };

        self.decrement_balance(from, amount)?;
        self.increment_balance(to, amount)?;

        let next_child = self
            .child_counters
            .entry(lineage_root.clone())
            .and_modify(|counter| *counter += 1)
            .or_insert(1);

        let child_id =
            ComputeReceiptId::new(format!("{}::xfer::{}", lineage_root.as_str(), *next_child));

        let parent_lot = self
            .lots
            .get(receipt_id)
            .expect("parent lot exists after mutation");
        let child_receipt = ComputeReceipt {
            id: child_id.clone(),
            contributor,
            beneficiary: to,
            credits: amount,
            compute_units: parent_lot.receipt.compute_units,
            issued_at: parent_lot.receipt.issued_at,
            expires_at: parent_lot.receipt.expires_at,
            runtime_nonce: parent_lot.receipt.runtime_nonce,
        };

        self.lots.insert(
            child_id.clone(),
            ComputeCreditLot {
                receipt: child_receipt,
                owner: to,
                available: amount,
                reserved: Amount::ZERO,
                spent: Amount::ZERO,
                slashed: Amount::ZERO,
                status: ComputeCreditStatus::Minted,
                lineage_root,
                transfer_generation: parent_generation + 1,
            },
        );

        Ok(child_id)
    }

    pub fn reserve(
        &mut self,
        receipt_id: &ComputeReceiptId,
        owner: Address,
        amount: Amount,
    ) -> DefiResult<()> {
        let lot =
            self.lots
                .get_mut(receipt_id)
                .ok_or_else(|| DefiError::InvalidComputeReceipt {
                    reason: format!("unknown receipt lot {}", receipt_id.as_str()),
                })?;

        if lot.owner != owner {
            return Err(DefiError::Unauthorized {
                reason: "reserve source does not own receipt lot".into(),
            });
        }
        if amount.is_zero() {
            return Err(DefiError::InvalidAmount {
                reason: "reserve amount must be greater than zero".into(),
            });
        }
        if lot.available < amount {
            return Err(DefiError::InsufficientBalance {
                required: amount,
                available: lot.available,
            });
        }

        lot.available = lot
            .available
            .checked_sub(amount)
            .ok_or_else(|| DefiError::Overflow {
                operation: "compute-credit reserve available".into(),
            })?;
        lot.reserved = lot
            .reserved
            .checked_add(amount)
            .ok_or_else(|| DefiError::Overflow {
                operation: "compute-credit reserve held".into(),
            })?;
        lot.status = status_for(lot.available, lot.reserved, lot.spent, lot.slashed);

        let current_reserved = self.reserved_balance_of(owner);
        self.reserved_balances.insert(
            owner,
            current_reserved
                .checked_add(amount)
                .ok_or_else(|| DefiError::Overflow {
                    operation: "compute-credit account reserve".into(),
                })?,
        );

        Ok(())
    }

    pub fn spend_reserved(
        &mut self,
        receipt_id: &ComputeReceiptId,
        owner: Address,
        amount: Amount,
    ) -> DefiResult<()> {
        let lot =
            self.lots
                .get_mut(receipt_id)
                .ok_or_else(|| DefiError::InvalidComputeReceipt {
                    reason: format!("unknown receipt lot {}", receipt_id.as_str()),
                })?;

        if lot.owner != owner {
            return Err(DefiError::Unauthorized {
                reason: "spend source does not own receipt lot".into(),
            });
        }
        if lot.reserved < amount {
            return Err(DefiError::InsufficientReservedBalance {
                required: amount,
                available: lot.reserved,
            });
        }

        lot.reserved = lot
            .reserved
            .checked_sub(amount)
            .ok_or_else(|| DefiError::Overflow {
                operation: "compute-credit reserved spend".into(),
            })?;
        lot.spent = lot
            .spent
            .checked_add(amount)
            .ok_or_else(|| DefiError::Overflow {
                operation: "compute-credit spent accumulation".into(),
            })?;
        lot.status = status_for(lot.available, lot.reserved, lot.spent, lot.slashed);

        self.decrement_reserved_balance(owner, amount)?;
        self.decrement_balance(owner, amount)?;
        self.total_supply =
            self.total_supply
                .checked_sub(amount)
                .ok_or_else(|| DefiError::Overflow {
                    operation: "compute-credit total supply spend".into(),
                })?;

        Ok(())
    }

    pub fn burn(
        &mut self,
        receipt_id: &ComputeReceiptId,
        owner: Address,
        amount: Amount,
    ) -> DefiResult<()> {
        let lot =
            self.lots
                .get_mut(receipt_id)
                .ok_or_else(|| DefiError::InvalidComputeReceipt {
                    reason: format!("unknown receipt lot {}", receipt_id.as_str()),
                })?;

        if lot.owner != owner {
            return Err(DefiError::Unauthorized {
                reason: "burn source does not own receipt lot".into(),
            });
        }
        if lot.available < amount {
            return Err(DefiError::InsufficientBalance {
                required: amount,
                available: lot.available,
            });
        }

        lot.available = lot
            .available
            .checked_sub(amount)
            .ok_or_else(|| DefiError::Overflow {
                operation: "compute-credit burn lot".into(),
            })?;
        lot.status = status_for(lot.available, lot.reserved, lot.spent, lot.slashed);
        self.decrement_balance(owner, amount)?;
        self.total_supply =
            self.total_supply
                .checked_sub(amount)
                .ok_or_else(|| DefiError::Overflow {
                    operation: "compute-credit total supply burn".into(),
                })?;

        Ok(())
    }

    pub fn slash(
        &mut self,
        receipt_id: &ComputeReceiptId,
        owner: Address,
        amount: Amount,
    ) -> DefiResult<()> {
        let lot =
            self.lots
                .get_mut(receipt_id)
                .ok_or_else(|| DefiError::InvalidComputeReceipt {
                    reason: format!("unknown receipt lot {}", receipt_id.as_str()),
                })?;

        if lot.owner != owner {
            return Err(DefiError::Unauthorized {
                reason: "slash source does not own receipt lot".into(),
            });
        }
        if lot.available < amount {
            return Err(DefiError::InsufficientBalance {
                required: amount,
                available: lot.available,
            });
        }

        lot.available = lot
            .available
            .checked_sub(amount)
            .ok_or_else(|| DefiError::Overflow {
                operation: "compute-credit slash lot".into(),
            })?;
        lot.slashed = lot
            .slashed
            .checked_add(amount)
            .ok_or_else(|| DefiError::Overflow {
                operation: "compute-credit slashed accumulation".into(),
            })?;
        lot.status = status_for(lot.available, lot.reserved, lot.spent, lot.slashed);
        self.decrement_balance(owner, amount)?;
        self.total_supply =
            self.total_supply
                .checked_sub(amount)
                .ok_or_else(|| DefiError::Overflow {
                    operation: "compute-credit total supply slash".into(),
                })?;

        Ok(())
    }

    pub fn expire(&mut self, now: Timestamp) -> DefiResult<Vec<ComputeReceiptId>> {
        let expired_ids: Vec<ComputeReceiptId> = self
            .lots
            .iter()
            .filter(|(_, lot)| {
                lot.receipt.expires_at <= now && (lot.available.0 > 0 || lot.reserved.0 > 0)
            })
            .map(|(id, _)| id.clone())
            .collect();

        for receipt_id in &expired_ids {
            let (owner, available, reserved) = {
                let lot = self.lots.get(receipt_id).expect("receipt exists");
                (lot.owner, lot.available, lot.reserved)
            };

            if !available.is_zero() {
                self.decrement_balance(owner, available)?;
            }
            if !reserved.is_zero() {
                self.decrement_reserved_balance(owner, reserved)?;
                self.decrement_balance(owner, reserved)?;
            }

            let lot = self.lots.get_mut(receipt_id).expect("receipt exists");
            let burned = available
                .checked_add(reserved)
                .ok_or_else(|| DefiError::Overflow {
                    operation: "compute-credit expire total".into(),
                })?;
            self.total_supply =
                self.total_supply
                    .checked_sub(burned)
                    .ok_or_else(|| DefiError::Overflow {
                        operation: "compute-credit total supply expire".into(),
                    })?;
            lot.available = Amount::ZERO;
            lot.reserved = Amount::ZERO;
            lot.status = ComputeCreditStatus::Expired;
        }

        Ok(expired_ids)
    }

    pub fn validate_receipt(&self, receipt: &ComputeReceipt) -> DefiResult<()> {
        if receipt.id.as_str().is_empty() {
            return Err(DefiError::InvalidComputeReceipt {
                reason: "receipt id must not be empty".into(),
            });
        }
        if receipt.credits.is_zero() {
            return Err(DefiError::InvalidComputeReceipt {
                reason: "receipt credits must be greater than zero".into(),
            });
        }
        if receipt.compute_units == 0 {
            return Err(DefiError::InvalidComputeReceipt {
                reason: "compute units must be greater than zero".into(),
            });
        }
        if receipt.contributor == Address::zero() || receipt.beneficiary == Address::zero() {
            return Err(DefiError::InvalidComputeReceipt {
                reason: "receipt participants must be non-zero addresses".into(),
            });
        }
        if receipt.expires_at <= receipt.issued_at {
            return Err(DefiError::InvalidComputeReceipt {
                reason: "receipt expiration must be after issuance".into(),
            });
        }
        Ok(())
    }

    fn increment_balance(&mut self, owner: Address, amount: Amount) -> DefiResult<()> {
        let updated =
            self.balance_of(owner)
                .checked_add(amount)
                .ok_or_else(|| DefiError::Overflow {
                    operation: "compute-credit account balance increment".into(),
                })?;
        self.balances.insert(owner, updated);
        Ok(())
    }

    fn decrement_balance(&mut self, owner: Address, amount: Amount) -> DefiResult<()> {
        let current = self.balance_of(owner);
        let updated = current
            .checked_sub(amount)
            .ok_or(DefiError::InsufficientBalance {
                required: amount,
                available: current,
            })?;
        self.balances.insert(owner, updated);
        Ok(())
    }

    fn decrement_reserved_balance(&mut self, owner: Address, amount: Amount) -> DefiResult<()> {
        let current = self.reserved_balance_of(owner);
        let updated =
            current
                .checked_sub(amount)
                .ok_or(DefiError::InsufficientReservedBalance {
                    required: amount,
                    available: current,
                })?;
        self.reserved_balances.insert(owner, updated);
        Ok(())
    }
}

fn status_for(
    available: Amount,
    reserved: Amount,
    spent: Amount,
    slashed: Amount,
) -> ComputeCreditStatus {
    if available.is_zero() && reserved.is_zero() && !spent.is_zero() {
        ComputeCreditStatus::Spent
    } else if !slashed.is_zero() && available.is_zero() && reserved.is_zero() {
        ComputeCreditStatus::Slashed
    } else if !reserved.is_zero() && available.is_zero() {
        ComputeCreditStatus::FullyReserved
    } else if !reserved.is_zero() {
        ComputeCreditStatus::PartiallyReserved
    } else if !spent.is_zero() {
        ComputeCreditStatus::PartiallySpent
    } else {
        ComputeCreditStatus::Minted
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(byte: u8) -> Address {
        Address([byte; 32])
    }

    fn receipt(
        id: &str,
        contributor: Address,
        beneficiary: Address,
        credits: u128,
    ) -> ComputeReceipt {
        ComputeReceipt {
            id: ComputeReceiptId::new(id),
            contributor,
            beneficiary,
            credits: Amount::new(credits),
            compute_units: 10_000,
            issued_at: Timestamp::new(10),
            expires_at: Timestamp::new(100),
            runtime_nonce: 7,
        }
    }

    #[test]
    fn mints_credits_once_per_receipt() {
        let mut ledger = ComputeCreditLedger::new();
        let r = receipt("receipt-1", addr(1), addr(2), 500);

        ledger.mint_from_receipt(r.clone()).unwrap();
        let duplicate = ledger.mint_from_receipt(r).unwrap_err();
        assert!(matches!(
            duplicate,
            DefiError::DuplicateComputeReceipt { .. }
        ));
        assert_eq!(ledger.total_supply(), Amount::new(500));
        assert_eq!(ledger.balance_of(addr(2)), Amount::new(500));
    }

    #[test]
    fn reserves_and_spends_credits_deterministically() {
        let mut ledger = ComputeCreditLedger::new();
        let r = receipt("receipt-2", addr(1), addr(2), 400);
        ledger.mint_from_receipt(r.clone()).unwrap();

        ledger.reserve(&r.id, addr(2), Amount::new(150)).unwrap();
        assert_eq!(ledger.balance_of(addr(2)), Amount::new(400));
        assert_eq!(ledger.reserved_balance_of(addr(2)), Amount::new(150));

        ledger
            .spend_reserved(&r.id, addr(2), Amount::new(100))
            .unwrap();

        let lot = ledger.lot(&r.id).unwrap();
        assert_eq!(lot.available, Amount::new(250));
        assert_eq!(lot.reserved, Amount::new(50));
        assert_eq!(lot.spent, Amount::new(100));
        assert_eq!(ledger.balance_of(addr(2)), Amount::new(300));
        assert_eq!(ledger.total_supply(), Amount::new(300));
    }

    #[test]
    fn transfers_create_child_lots_and_block_circular_routes() {
        let mut ledger = ComputeCreditLedger::new();
        let r = receipt("receipt-3", addr(1), addr(2), 500);
        ledger.mint_from_receipt(r.clone()).unwrap();

        let child_id = ledger
            .transfer(&r.id, addr(2), addr(3), Amount::new(200))
            .unwrap();

        assert_eq!(ledger.balance_of(addr(2)), Amount::new(300));
        assert_eq!(ledger.balance_of(addr(3)), Amount::new(200));
        assert_eq!(ledger.lot(&child_id).unwrap().owner, addr(3));

        let abuse = ledger
            .transfer(&child_id, addr(3), addr(1), Amount::new(50))
            .unwrap_err();
        assert!(matches!(abuse, DefiError::ComputeCreditAbuse { .. }));
    }

    #[test]
    fn burns_slashes_and_expires_remaining_supply() {
        let mut ledger = ComputeCreditLedger::new();
        let r = receipt("receipt-4", addr(1), addr(2), 600);
        ledger.mint_from_receipt(r.clone()).unwrap();

        ledger.burn(&r.id, addr(2), Amount::new(100)).unwrap();
        ledger.slash(&r.id, addr(2), Amount::new(50)).unwrap();
        let expired = ledger.expire(Timestamp::new(101)).unwrap();

        assert_eq!(expired, vec![r.id.clone()]);
        assert_eq!(ledger.balance_of(addr(2)), Amount::ZERO);
        assert_eq!(ledger.total_supply(), Amount::ZERO);
        assert_eq!(
            ledger.lot(&r.id).unwrap().status,
            ComputeCreditStatus::Expired
        );
    }

    #[test]
    fn rejects_invalid_receipts_and_self_transfers() {
        let mut ledger = ComputeCreditLedger::new();
        let invalid = ComputeReceipt {
            id: ComputeReceiptId::new(""),
            contributor: Address::zero(),
            beneficiary: addr(2),
            credits: Amount::ZERO,
            compute_units: 0,
            issued_at: Timestamp::new(10),
            expires_at: Timestamp::new(5),
            runtime_nonce: 0,
        };
        assert!(matches!(
            ledger.mint_from_receipt(invalid),
            Err(DefiError::InvalidComputeReceipt { .. })
        ));

        let valid = receipt("receipt-5", addr(1), addr(2), 250);
        ledger.mint_from_receipt(valid.clone()).unwrap();

        let wash = ledger
            .transfer(&valid.id, addr(2), addr(2), Amount::new(10))
            .unwrap_err();
        assert!(matches!(wash, DefiError::ComputeCreditAbuse { .. }));
    }

    #[test]
    fn simulation_pressure_keeps_supply_below_minted_total_after_spend_and_slash() {
        let mut ledger = ComputeCreditLedger::new();
        let beneficiary = addr(9);
        let mut minted_total = Amount::ZERO;

        for idx in 0..5 {
            let r = receipt(&format!("receipt-sim-{idx}"), addr(1), beneficiary, 1_000);
            minted_total = minted_total.checked_add(r.credits).unwrap();
            ledger.mint_from_receipt(r.clone()).unwrap();
            ledger
                .reserve(&r.id, beneficiary, Amount::new(200))
                .unwrap();
            ledger
                .spend_reserved(&r.id, beneficiary, Amount::new(150))
                .unwrap();
            ledger.slash(&r.id, beneficiary, Amount::new(100)).unwrap();
        }

        assert!(ledger.total_supply() < minted_total);
        assert_eq!(ledger.balance_of(beneficiary), Amount::new(3_750));
    }
}
