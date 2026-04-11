use thiserror::Error;

#[derive(Debug, Clone, Error)]
pub enum RevenueError {
    #[error("no eligible contributors for distribution")]
    NoEligibleContributors,
    #[error("distribution interval not reached: current={current}, next={next}")]
    DistributionTooEarly { current: u64, next: u64 },
    #[error("contributor not found: {0:?}")]
    ContributorNotFound(wrfcoin_defi_common::types::Address),
    #[error("arithmetic overflow in {operation}")]
    Overflow { operation: String },
    #[error("defi error: {0}")]
    Defi(#[from] wrfcoin_defi_common::error::DefiError),
}

pub type RevenueResult<T> = Result<T, RevenueError>;
