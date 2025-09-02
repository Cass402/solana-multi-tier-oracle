use anchor_lang::prelude::*;

#[error_code]
pub enum StateError {
    #[msg("Active feed count exceeds maximum limit")]
    TooManyActiveFeeds,
    #[msg("Excessive liquidity provider concentration detected")]
    ExcessiveLpConcentration,
    #[msg("Price manipulation detected")]
    ManipulationDetected,
    #[msg("Caller does not have sufficient permissions for this operation")]
    InsufficientPermissions,
    #[msg("Caller is not authorized to perform this operation")]
    UnauthorizedCaller,
    #[msg("Too many active multisig members")]
    TooManyActiveMembers,
}