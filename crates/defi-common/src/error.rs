use crate::types::{Amount, AssetId, PoolId};
use thiserror::Error;

/// Unified error type for all DeFi protocol operations.
#[derive(Debug, Clone, Error)]
pub enum DefiError {
    #[error("insufficient balance: required {required}, available {available}")]
    InsufficientBalance { required: Amount, available: Amount },

    #[error("insufficient collateral: health factor {health_factor_bps} bps below threshold")]
    InsufficientCollateral { health_factor_bps: u64 },

    #[error("insufficient liquidity in pool: requested {requested}, available {available}")]
    InsufficientLiquidity { requested: Amount, available: Amount },

    #[error("invalid amount: {reason}")]
    InvalidAmount { reason: String },

    #[error("invalid asset: {asset_id}")]
    InvalidAsset { asset_id: AssetId },

    #[error("invalid pool: {pool_id}")]
    InvalidPool { pool_id: PoolId },

    #[error("slippage exceeded: expected minimum {minimum_out}, got {actual_out}")]
    Slippage {
        minimum_out: Amount,
        actual_out: Amount,
    },

    #[error("oracle error: {message}")]
    OracleError { message: String },

    #[error("unauthorized: {reason}")]
    Unauthorized { reason: String },

    #[error("arithmetic overflow in {operation}")]
    Overflow { operation: String },

    #[error("duplicate compute receipt: {receipt_id}")]
    DuplicateComputeReceipt { receipt_id: String },

    #[error("invalid compute receipt: {reason}")]
    InvalidComputeReceipt { reason: String },

    #[error("compute credit abuse detected: {reason}")]
    ComputeCreditAbuse { reason: String },

    #[error("insufficient reserved balance: required {required}, available {available}")]
    InsufficientReservedBalance { required: Amount, available: Amount },
}

/// Convenience alias for DeFi results.
pub type DefiResult<T> = Result<T, DefiError>;
