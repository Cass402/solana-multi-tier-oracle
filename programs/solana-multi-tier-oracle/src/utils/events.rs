use anchor_lang::prelude::*;
use crate::state::price_feed::SourceType;

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

#[event]
pub struct PriceFeedRegistered {
    pub oracle: Pubkey,
    pub feed_address: Pubkey,
    pub source_type: SourceType,
    pub weight: u16,
    pub feed_index: u32,
    pub total_weight: u32,
    pub timestamp: i64,
}

#[event]
pub struct PriceUpdated {
    pub oracle: Pubkey,
    pub price: i128,
    pub confidence: u64,
    pub timestamp: i64,
    pub twap_window: u32,
    pub raydium_pools_used: u8,
    pub observed_manipulation_score: u32,
    pub raydium_network_mainnet: u8, // Network flag for operational visibility
}

#[event]
pub struct CircuitBreakerTriggered {
    pub oracle: Pubkey,
    pub triggered_by: Pubkey,
    pub timestamp: i64,
    pub manipulation_score: u32,
    pub reason_hash: [u8; 32],
}

#[event]
pub struct TwapMetrics {
    pub oracle: Pubkey,
    pub data_points_used: u16,
    pub covered_time_span: u64,
    pub timestamp: i64,
}

#[event]
pub struct SaturationWarning {
    pub oracle: Pubkey,
    pub operation: String,
    pub timestamp: i64,
    pub data_points_processed: u32,
}