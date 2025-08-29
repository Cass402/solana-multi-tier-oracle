use crate::utils::constants::{MAX_PRICE_FEEDS, MAX_HISTORICAL_CHUNKS};
use crate::state::price_feed::PriceFeed;
use anchor_lang::prelude::*;
use bytemuck::{Pod, Zeroable};

#[account(zero_copy)]
#[repr(C)]
pub struct OracleState {
    pub authority: Pubkey,
    pub version: Version,
    pub flags: StateFlags,

    pub current_price: PriceData,
    pub last_update: u64,
    pub price_feeds: [PriceFeed; MAX_PRICE_FEEDS],

    pub twap_window: u16,
    pub current_chunk_index: u16,
    pub max_chunk_size: u16,

    pub confidence_threshold: u16,
    pub manipulation_threshold: u16,

    pub active_feed_count: u8,
    pub bump: u8,

    pub historical_chunks: [Pubkey; MAX_HISTORICAL_CHUNKS],

    pub _padding: [u8; 4],

    pub reserved: [u64; 8], // reserved for future use
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Pod, Zeroable, Default)]
#[repr(transparent)]
pub struct StateFlags(u32);

impl StateFlags {
    // Type-safe flag values
    pub const CIRCUIT_BREAKER_ENABLED: Self = Self(0b0000_0001);
    pub const EMERGENCY_MODE: Self          = Self(0b0000_0010);
    pub const UPGRADE_LOCKED: Self          = Self(0b0000_0100);
    pub const MAINTENANCE_MODE: Self        = Self(0b0000_1000);
    pub const TWAP_ENABLED: Self            = Self(0b0001_0000);

    pub const VALID_MASK: u32 = Self::CIRCUIT_BREAKER_ENABLED.0
        | Self::EMERGENCY_MODE.0
        | Self::UPGRADE_LOCKED.0
        | Self::MAINTENANCE_MODE.0
        | Self::TWAP_ENABLED.0;

    #[inline(always)] pub const fn new() -> Self { Self(0) }

    #[inline(always)] 
    pub fn has(self, flag: Self) -> bool { 
        (self.0 & flag.0) != 0 
    }

    #[inline(always)] 
    pub fn set(&mut self, flag: Self) {
        self.0 |= flag.0; 
    }

    #[inline(always)] 
    pub fn clear(&mut self, flag: Self) { 
        self.0 &= !flag.0; 
    }

    #[inline(always)] 
    pub fn toggle(&mut self, flag: Self)  { 
        self.0 ^= flag.0; 
    }

    #[inline(always)] 
    pub fn set_to(&mut self, flag: Self, on: bool) {
        if on { self.set(flag) } else { self.clear(flag) }
    }

    // Convenience specific accessors
    #[inline(always)] 
    pub fn is_circuit_breaker_enabled(self) -> bool { 
        self.has(Self::CIRCUIT_BREAKER_ENABLED) 
    }

    #[inline(always)] 
    pub fn is_emergency_mode(self) -> bool { 
        self.has(Self::EMERGENCY_MODE) 
    }

    #[inline(always)] 
    pub fn is_upgrade_locked(self) -> bool { 
        self.has(Self::UPGRADE_LOCKED) 
    }

    #[inline(always)] 
    pub fn is_maintenance_mode(self) -> bool { 
        self.has(Self::MAINTENANCE_MODE) 
    }

    #[inline(always)] 
    pub fn is_twap_enabled(self) -> bool { 
        self.has(Self::TWAP_ENABLED) 
    }

    // Conversions for account IO
    #[inline(always)] 
    pub const fn as_u32(self) -> u32 { 
        self.0 
    }

    #[inline(always)] 
    pub const fn from_u32_truncate(value: u32) -> Self {
        // lenient: drop unknown bits for forward-compat reads
        Self(value & Self::VALID_MASK)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
#[repr(C)]
pub struct Version {
    pub major: u8,
    pub minor: u8,
    pub patch: u8,
    pub _padding: u8, // Padding for alignment
}

#[derive(Clone, Copy, Debug, Pod, Zeroable)]
#[repr(C)]
pub struct PriceData {
    pub price: i64,
    pub conf: u64,
    pub timestamp: u64,
    pub expo: i32,
    pub _padding: [u8; 4], // Padding for alignment
}
