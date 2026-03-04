use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use wrfcoin_defi_common::error::{DefiError, DefiResult};
use wrfcoin_defi_common::types::{Address, Amount, PoolId, Timestamp};

const BPS_SCALE: u16 = 10_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum QualityVerificationOutcome {
    Pending,
    Verified { quality_score_bps: u16 },
    FraudDetected,
}

pub trait QualityOutcomeSignalSource {
    fn quality_outcome_for_reward(
        &self,
        beneficiary: Address,
        pool_id: PoolId,
        reward_amount: Amount,
        reward_timestamp: Timestamp,
    ) -> QualityVerificationOutcome;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RewardEscrowPolicy {
    pub policy_version: u16,
    pub min_quality_score_bps: u16,
    pub forfeit_on_low_quality: bool,
    pub forfeit_on_fraud: bool,
}

impl Default for RewardEscrowPolicy {
    fn default() -> Self {
        Self {
            policy_version: 1,
            min_quality_score_bps: 6_500,
            forfeit_on_low_quality: true,
            forfeit_on_fraud: true,
        }
    }
}

impl RewardEscrowPolicy {
    pub fn validate(&self) -> DefiResult<()> {
        if self.policy_version == 0 {
            return Err(DefiError::InvalidAmount {
                reason: "policy_version must be >= 1".into(),
            });
        }
        if self.min_quality_score_bps > BPS_SCALE {
            return Err(DefiError::InvalidAmount {
                reason: "min_quality_score_bps must be <= 10_000".into(),
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ForfeitReason {
    LowQuality,
    FraudDetected,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EscrowedReward {
    pub escrow_id: u64,
    pub beneficiary: Address,
    pub pool_id: PoolId,
    pub amount: Amount,
    pub created_at: Timestamp,
    pub last_updated_at: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EscrowSettlement {
    Held(EscrowedReward),
    Released {
        amount: Amount,
        escrow_id: Option<u64>,
    },
    Forfeited {
        amount: Amount,
        escrow_id: Option<u64>,
        reason: ForfeitReason,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct EscrowTelemetry {
    pub total_rewards_observed: Amount,
    pub rewards_released: Amount,
    pub rewards_released_after_verification: Amount,
    pub rewards_forfeited: Amount,
    pub rewards_currently_escrowed: Amount,
    pub settled_verification_events: u64,
}

impl Default for EscrowTelemetry {
    fn default() -> Self {
        Self {
            total_rewards_observed: Amount::ZERO,
            rewards_released: Amount::ZERO,
            rewards_released_after_verification: Amount::ZERO,
            rewards_forfeited: Amount::ZERO,
            rewards_currently_escrowed: Amount::ZERO,
            settled_verification_events: 0,
        }
    }
}

impl EscrowTelemetry {
    pub fn released_after_verification_share_bps(&self) -> u16 {
        if self.rewards_released.is_zero() {
            return 0;
        }
        self.rewards_released_after_verification
            .0
            .saturating_mul(BPS_SCALE as u128)
            .saturating_div(self.rewards_released.0)
            .min(BPS_SCALE as u128) as u16
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RewardEscrowBook {
    pub policy: RewardEscrowPolicy,
    pub next_escrow_id: u64,
    pub escrowed: BTreeMap<u64, EscrowedReward>,
    pub telemetry: EscrowTelemetry,
}

impl RewardEscrowBook {
    pub fn with_policy(policy: RewardEscrowPolicy) -> DefiResult<Self> {
        policy.validate()?;
        Ok(Self {
            policy,
            next_escrow_id: 1,
            escrowed: BTreeMap::new(),
            telemetry: EscrowTelemetry::default(),
        })
    }

    pub fn set_policy(&mut self, policy: RewardEscrowPolicy) -> DefiResult<()> {
        policy.validate()?;
        self.policy = policy;
        Ok(())
    }

    pub fn telemetry(&self) -> EscrowTelemetry {
        self.telemetry
    }

    pub fn escrowed_count(&self) -> usize {
        self.escrowed.len()
    }

    pub fn escrowed_reward(&self, escrow_id: u64) -> Option<&EscrowedReward> {
        self.escrowed.get(&escrow_id)
    }

    pub fn record_reward(
        &mut self,
        beneficiary: Address,
        pool_id: PoolId,
        amount: Amount,
        timestamp: Timestamp,
        outcome: QualityVerificationOutcome,
    ) -> DefiResult<EscrowSettlement> {
        if amount.is_zero() {
            return Ok(EscrowSettlement::Released {
                amount,
                escrow_id: None,
            });
        }

        self.telemetry.total_rewards_observed = checked_add_amount(
            self.telemetry.total_rewards_observed,
            amount,
            "escrow telemetry total_rewards_observed",
        )?;

        match outcome {
            QualityVerificationOutcome::Pending => self.hold_in_escrow(beneficiary, pool_id, amount, timestamp),
            QualityVerificationOutcome::FraudDetected => {
                self.resolve_fraud(amount, None, beneficiary, pool_id, timestamp)
            }
            QualityVerificationOutcome::Verified { quality_score_bps } => {
                self.resolve_verified_score(
                    amount,
                    quality_score_bps,
                    None,
                    beneficiary,
                    pool_id,
                    timestamp,
                )
            }
        }
    }

    pub fn settle_escrow(
        &mut self,
        escrow_id: u64,
        outcome: QualityVerificationOutcome,
        settled_at: Timestamp,
    ) -> DefiResult<EscrowSettlement> {
        let mut entry = self
            .escrowed
            .get(&escrow_id)
            .cloned()
            .ok_or(DefiError::InvalidAmount {
                reason: format!("unknown escrow_id {escrow_id}"),
            })?;
        entry.last_updated_at = settled_at;

        match outcome {
            QualityVerificationOutcome::Pending => {
                self.escrowed.insert(escrow_id, entry.clone());
                Ok(EscrowSettlement::Held(entry))
            }
            QualityVerificationOutcome::FraudDetected => {
                self.escrowed.remove(&escrow_id);
                self.telemetry.rewards_currently_escrowed = checked_sub_amount(
                    self.telemetry.rewards_currently_escrowed,
                    entry.amount,
                    "escrow telemetry rewards_currently_escrowed remove fraud",
                )?;
                self.resolve_fraud(
                    entry.amount,
                    Some(escrow_id),
                    entry.beneficiary,
                    entry.pool_id,
                    settled_at,
                )
            }
            QualityVerificationOutcome::Verified { quality_score_bps } => {
                if quality_score_bps > BPS_SCALE {
                    return Err(DefiError::InvalidAmount {
                        reason: "quality_score_bps must be <= 10_000".into(),
                    });
                }
                if quality_score_bps >= self.policy.min_quality_score_bps {
                    self.escrowed.remove(&escrow_id);
                    self.telemetry.rewards_currently_escrowed = checked_sub_amount(
                        self.telemetry.rewards_currently_escrowed,
                        entry.amount,
                        "escrow telemetry rewards_currently_escrowed remove release",
                    )?;
                    self.telemetry.rewards_released = checked_add_amount(
                        self.telemetry.rewards_released,
                        entry.amount,
                        "escrow telemetry rewards_released",
                    )?;
                    self.telemetry.rewards_released_after_verification = checked_add_amount(
                        self.telemetry.rewards_released_after_verification,
                        entry.amount,
                        "escrow telemetry rewards_released_after_verification",
                    )?;
                    self.telemetry.settled_verification_events =
                        self.telemetry.settled_verification_events.saturating_add(1);
                    Ok(EscrowSettlement::Released {
                        amount: entry.amount,
                        escrow_id: Some(escrow_id),
                    })
                } else if self.policy.forfeit_on_low_quality {
                    self.escrowed.remove(&escrow_id);
                    self.telemetry.rewards_currently_escrowed = checked_sub_amount(
                        self.telemetry.rewards_currently_escrowed,
                        entry.amount,
                        "escrow telemetry rewards_currently_escrowed remove forfeit",
                    )?;
                    self.telemetry.rewards_forfeited = checked_add_amount(
                        self.telemetry.rewards_forfeited,
                        entry.amount,
                        "escrow telemetry rewards_forfeited low quality",
                    )?;
                    self.telemetry.settled_verification_events =
                        self.telemetry.settled_verification_events.saturating_add(1);
                    Ok(EscrowSettlement::Forfeited {
                        amount: entry.amount,
                        escrow_id: Some(escrow_id),
                        reason: ForfeitReason::LowQuality,
                    })
                } else {
                    self.escrowed.insert(escrow_id, entry.clone());
                    Ok(EscrowSettlement::Held(entry))
                }
            }
        }
    }

    fn hold_in_escrow(
        &mut self,
        beneficiary: Address,
        pool_id: PoolId,
        amount: Amount,
        timestamp: Timestamp,
    ) -> DefiResult<EscrowSettlement> {
        let escrow_id = self.next_escrow_id;
        self.next_escrow_id = self.next_escrow_id.saturating_add(1);

        let reward = EscrowedReward {
            escrow_id,
            beneficiary,
            pool_id,
            amount,
            created_at: timestamp,
            last_updated_at: timestamp,
        };
        self.escrowed.insert(escrow_id, reward.clone());
        self.telemetry.rewards_currently_escrowed = checked_add_amount(
            self.telemetry.rewards_currently_escrowed,
            amount,
            "escrow telemetry rewards_currently_escrowed add",
        )?;
        Ok(EscrowSettlement::Held(reward))
    }

    fn resolve_fraud(
        &mut self,
        amount: Amount,
        escrow_id: Option<u64>,
        beneficiary: Address,
        pool_id: PoolId,
        timestamp: Timestamp,
    ) -> DefiResult<EscrowSettlement> {
        if self.policy.forfeit_on_fraud {
            self.telemetry.rewards_forfeited = checked_add_amount(
                self.telemetry.rewards_forfeited,
                amount,
                "escrow telemetry rewards_forfeited fraud",
            )?;
            self.telemetry.settled_verification_events =
                self.telemetry.settled_verification_events.saturating_add(1);
            Ok(EscrowSettlement::Forfeited {
                amount,
                escrow_id,
                reason: ForfeitReason::FraudDetected,
            })
        } else {
            self.hold_in_escrow(beneficiary, pool_id, amount, timestamp)
        }
    }

    fn resolve_verified_score(
        &mut self,
        amount: Amount,
        quality_score_bps: u16,
        escrow_id: Option<u64>,
        beneficiary: Address,
        pool_id: PoolId,
        timestamp: Timestamp,
    ) -> DefiResult<EscrowSettlement> {
        if quality_score_bps > BPS_SCALE {
            return Err(DefiError::InvalidAmount {
                reason: "quality_score_bps must be <= 10_000".into(),
            });
        }
        if quality_score_bps >= self.policy.min_quality_score_bps {
            self.telemetry.rewards_released = checked_add_amount(
                self.telemetry.rewards_released,
                amount,
                "escrow telemetry rewards_released",
            )?;
            self.telemetry.rewards_released_after_verification = checked_add_amount(
                self.telemetry.rewards_released_after_verification,
                amount,
                "escrow telemetry rewards_released_after_verification",
            )?;
            self.telemetry.settled_verification_events =
                self.telemetry.settled_verification_events.saturating_add(1);
            Ok(EscrowSettlement::Released { amount, escrow_id })
        } else if self.policy.forfeit_on_low_quality {
            self.telemetry.rewards_forfeited = checked_add_amount(
                self.telemetry.rewards_forfeited,
                amount,
                "escrow telemetry rewards_forfeited low quality",
            )?;
            self.telemetry.settled_verification_events =
                self.telemetry.settled_verification_events.saturating_add(1);
            Ok(EscrowSettlement::Forfeited {
                amount,
                escrow_id,
                reason: ForfeitReason::LowQuality,
            })
        } else {
            self.hold_in_escrow(beneficiary, pool_id, amount, timestamp)
        }
    }
}

impl Default for RewardEscrowBook {
    fn default() -> Self {
        Self::with_policy(RewardEscrowPolicy::default())
            .expect("default reward escrow policy must be valid")
    }
}

fn checked_add_amount(lhs: Amount, rhs: Amount, operation: &str) -> DefiResult<Amount> {
    lhs.checked_add(rhs).ok_or(DefiError::Overflow {
        operation: operation.into(),
    })
}

fn checked_sub_amount(lhs: Amount, rhs: Amount, operation: &str) -> DefiResult<Amount> {
    lhs.checked_sub(rhs).ok_or(DefiError::Overflow {
        operation: operation.into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(n: u8) -> Address {
        let mut bytes = [0u8; 32];
        bytes[0] = n;
        Address::from_bytes(bytes)
    }

    #[test]
    fn pending_reward_is_held_until_verification_settles() {
        let mut book = RewardEscrowBook::default();
        let held = book
            .record_reward(
                addr(1),
                PoolId::new(0),
                Amount::new(250),
                Timestamp::new(100),
                QualityVerificationOutcome::Pending,
            )
            .unwrap();

        let escrow_id = match held {
            EscrowSettlement::Held(entry) => {
                assert_eq!(entry.amount, Amount::new(250));
                entry.escrow_id
            }
            _ => panic!("expected reward to be held"),
        };

        let settled = book
            .settle_escrow(
                escrow_id,
                QualityVerificationOutcome::Verified {
                    quality_score_bps: 7_500,
                },
                Timestamp::new(120),
            )
            .unwrap();

        match settled {
            EscrowSettlement::Released {
                amount,
                escrow_id: Some(id),
            } => {
                assert_eq!(amount, Amount::new(250));
                assert_eq!(id, escrow_id);
            }
            _ => panic!("expected escrow release"),
        }
        assert_eq!(book.escrowed_count(), 0);
        assert_eq!(
            book.telemetry().released_after_verification_share_bps(),
            BPS_SCALE
        );
    }

    #[test]
    fn low_quality_verification_forfeits_when_policy_enabled() {
        let mut book = RewardEscrowBook::default();
        let result = book
            .record_reward(
                addr(2),
                PoolId::new(1),
                Amount::new(300),
                Timestamp::new(1),
                QualityVerificationOutcome::Verified {
                    quality_score_bps: 4_000,
                },
            )
            .unwrap();

        match result {
            EscrowSettlement::Forfeited {
                amount,
                reason: ForfeitReason::LowQuality,
                ..
            } => assert_eq!(amount, Amount::new(300)),
            _ => panic!("expected low-quality forfeit"),
        }
        assert_eq!(book.telemetry().rewards_forfeited, Amount::new(300));
    }

    #[test]
    fn fraud_signal_forfeits_immediately() {
        let mut book = RewardEscrowBook::default();
        let result = book
            .record_reward(
                addr(3),
                PoolId::new(2),
                Amount::new(125),
                Timestamp::new(10),
                QualityVerificationOutcome::FraudDetected,
            )
            .unwrap();

        match result {
            EscrowSettlement::Forfeited {
                amount,
                reason: ForfeitReason::FraudDetected,
                ..
            } => assert_eq!(amount, Amount::new(125)),
            _ => panic!("expected fraud forfeit"),
        }
        assert_eq!(book.telemetry().settled_verification_events, 1);
    }
}
