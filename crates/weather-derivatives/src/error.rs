use thiserror::Error;
use wrfcoin_defi_common::types::Amount;

#[derive(Debug, Clone, Error)]
pub enum DerivativesError {
    #[error("future not found: {id}")]
    FutureNotFound { id: u64 },

    #[error("option not found: {id}")]
    OptionNotFound { id: u64 },

    #[error("already settled: id {id}")]
    AlreadySettled { id: u64 },

    #[error("not yet expired: settlement block {settlement_block}, current block {current_block}")]
    NotYetExpired {
        settlement_block: u64,
        current_block: u64,
    },

    #[error("insufficient margin: required {required}, available {available}")]
    InsufficientMargin {
        required: Amount,
        available: Amount,
    },

    #[error("position already under margin")]
    AlreadyUnderMargin,

    #[error("oracle reading not available for region {region} at block {block}")]
    OracleReadingUnavailable { region: String, block: u64 },

    #[error("invalid parameter: {reason}")]
    InvalidParameter { reason: String },

    #[error("arithmetic overflow in {operation}")]
    Overflow { operation: String },

    #[error("oracle error: {message}")]
    OracleError { message: String },
}

pub type DerivativesResult<T> = Result<T, DerivativesError>;
