use anchor_lang::prelude::*;

#[event]
pub struct OracleInitialized {
    pub oracle_state: Pubkey,
    pub asset_id: String,
    pub authority: Pubkey,
    pub emergency_admin: Pubkey,
    pub twap_window: u32,
    pub confidence_threshold: u16,
    pub manipulation_threshold: u16,
    pub governance_members: u8,
    pub multisig_threshold: u8,
}