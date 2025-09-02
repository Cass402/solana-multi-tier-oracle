/// State constants
pub const MAX_PRICE_FEEDS: usize = 16; // power of 2 for cache alignment
pub const BUFFER_SIZE: usize = 128; // power of 2 for efficiency
pub const MAX_HISTORICAL_CHUNKS: usize = 8; 
pub const MAX_LP_CONCENTRATION: u16 = 3_000; // 30%
pub const MAX_MULTISIG_MEMBERS: usize = 16;

/// PDA seed constants
pub const ORACLE_STATE_SEED: &[u8] = b"oracle_state";
pub const HISTORICAL_CHUNK_SEED: &[u8] = b"historical_chunk";
pub const GOVERNANCE: &[u8] = b"governance";