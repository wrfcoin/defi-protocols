use thiserror::Error;
use wrfcoin_defi_common::types::Amount;

/// All errors that can occur in the parametric insurance protocol.
#[derive(Debug, Clone, Error)]
pub enum InsuranceError {
    #[error("policy {policy_id} not found")]
    PolicyNotFound { policy_id: String },

    #[error("policy {policy_id} is not in Active state (current: {state})")]
    PolicyNotActive { policy_id: String, state: String },

    #[error("policy {policy_id} has already been settled")]
    AlreadySettled { policy_id: String },

    #[error("observation window has not yet closed for policy {policy_id}")]
    WindowNotClosed { policy_id: String },

    #[error("insufficient vault capital: required {required}, available {available}")]
    InsufficientCapital { required: Amount, available: Amount },

    #[error("invalid premium: {reason}")]
    InvalidPremium { reason: String },

    #[error("invalid payout: {reason}")]
    InvalidPayout { reason: String },

    #[error("risk limit exceeded: {reason}")]
    RiskLimitExceeded { reason: String },

    #[error("underwriter {underwriter} has insufficient shares to withdraw")]
    InsufficientShares { underwriter: String },

    #[error("vault capital ratio {actual_bps} bps below minimum {min_bps} bps")]
    CapitalRatioBelowMinimum { actual_bps: u64, min_bps: u64 },

    #[error("arithmetic overflow in {operation}")]
    Overflow { operation: String },

    #[error("invalid trigger: {reason}")]
    InvalidTrigger { reason: String },

    #[error("oracle reading is stale or missing")]
    MissingOracleReading,

    #[error("deposit amount must be non-zero")]
    ZeroDeposit,

    #[error("withdrawal amount must be non-zero")]
    ZeroWithdrawal,
}

/// Convenience alias.
pub type InsuranceResult<T> = Result<T, InsuranceError>;
