use anchor_lang::prelude::*;

#[error_code]
pub enum StateError {
    #[msg("Active feed count exceeds maximum limit")]
    TooManyActiveFeeds,
    #[msg("Excessive liquidity provider concentration detected")]
    ExcessiveLpConcentration,
    #[msg("Price manipulation detected")]
    ManipulationDetected,
}