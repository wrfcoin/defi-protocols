use thiserror::Error;

/// Errors that can occur within prediction market operations.
#[derive(Debug, Clone, Error, PartialEq)]
pub enum PredictionError {
    #[error("market not found: id={market_id}")]
    MarketNotFound { market_id: u64 },

    #[error("market already resolved: id={market_id}")]
    AlreadyResolved { market_id: u64 },

    #[error("market not yet resolvable: current_height={current_height}, resolution_height={resolution_height}")]
    ResolutionTooEarly {
        current_height: u64,
        resolution_height: u64,
    },

    #[error("market not resolved yet: id={market_id}")]
    NotResolved { market_id: u64 },

    #[error("invalid outcome index {outcome_index}: market has {num_outcomes} outcomes")]
    InvalidOutcome {
        outcome_index: usize,
        num_outcomes: usize,
    },

    #[error("invalid amount: {reason}")]
    InvalidAmount { reason: String },

    #[error("insufficient funds: required {required}, available {available}")]
    InsufficientFunds { required: u128, available: u128 },

    #[error("arithmetic overflow in {operation}")]
    Overflow { operation: String },

    #[error("no winners: cannot distribute payout")]
    NoWinners,

    #[error("LMSR computation error: {reason}")]
    LmsrError { reason: String },

    #[error("position not found for user on market {market_id} outcome {outcome_index}")]
    PositionNotFound { market_id: u64, outcome_index: usize },

    #[error("non-finite floating-point value in LMSR input")]
    NonFinite,
}

pub type PredictionResult<T> = Result<T, PredictionError>;
