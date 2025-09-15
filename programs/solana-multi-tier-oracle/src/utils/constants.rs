/// State constants
pub const MAX_PRICE_FEEDS: usize = 16; // power of 2 for cache alignment
pub const BUFFER_SIZE: usize = 128; // power of 2 for efficiency
pub const MAX_HISTORICAL_CHUNKS: usize = 8; 
pub const MAX_LP_CONCENTRATION: u16 = 3_000; // 30%
pub const MAX_MULTISIG_MEMBERS: usize = 16;
pub const MAX_TWAP_WINDOW: u32 = 345_600; // 96 hours in seconds
pub const MAX_CONFIDENCE_THRESHOLD: u16 = 10_000; // 100% in basis points
pub const MAX_MANIPULATION_THRESHOLD: u16 = 10_000; // 100% in basis points
pub const MAX_QUORUM_THRESHOLD: u16 = 10_000; // 100% in basis points
pub const DEFAULT_VETO_PERIOD: i64 = 86400; // 24 hours in seconds

/// Snapshot tracking constants for redemption quality control
/// (leverages existing HistoricalChunk infrastructure)
pub const MIN_SNAPSHOTS_24H: u16 = 12; // minimum snapshots required in 24 hours (kept for backward compatibility)
pub const MIN_TIME_SPAN_HOURS: u16 = 24; // minimum time coverage in hours (increased for safety)
pub const MAX_SNAPSHOTS_PER_HOUR: u16 = 4; // allow 4 per hour (matches 15-min intervals)
pub const MAX_HOURS: u16 = 96; 
pub const SECONDS_PER_HOUR: i64 = 3600;
pub const SECONDS_PER_24H: i64 = 86400;
pub const SECONDS_PER_72H: i64 = 259200; // 72 hours for TWAP validation
pub const SECONDS_PER_96H: i64 = 345600; // 96 hours maximum supported window

/// PDA seed constants
pub const ORACLE_STATE_SEED: &[u8] = b"oracle_state";
pub const HISTORICAL_CHUNK_SEED: &[u8] = b"historical_chunk";
pub const GOVERNANCE_SEED: &[u8] = b"governance";