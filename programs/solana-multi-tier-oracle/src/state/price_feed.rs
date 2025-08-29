use anchor_lang::prelude::*;
use bytemuck::{Pod, Zeroable};

#[derive(Clone, Copy, Debug, Pod, Zeroable)]
#[repr(C)]
pub struct PriceFeed {
    pub source_address: Pubkey,
    pub last_price: i64,
    pub last_conf: u64,
    pub last_update: u64,
    pub volume_24h: u64,
    pub liquidity_depth: u64,
    pub last_expo: i32,
    pub weight: u16,
    pub lp_concentration: u16,
    pub manipulation_score: u16,
    pub source_type: u8,
    pub flags: FeedFlags,
    pub _padding: [u8; 4], 
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Pod, Zeroable, Default)]
#[repr(transparent)]
pub struct FeedFlags(u8);

impl FeedFlags {
    // Type-safe flag values
    pub const ACTIVE: Self                = Self(0b0000_0001);
    pub const TRUSTED: Self               = Self(0b0000_0010);
    pub const STALE: Self                 = Self(0b0000_0100);
    pub const MANIPULATION_DETECTED: Self = Self(0b0000_1000);

    pub const VALID_MASK: u8 = Self::ACTIVE.0
        | Self::TRUSTED.0
        | Self::STALE.0
        | Self::MANIPULATION_DETECTED.0;

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
    pub fn is_active(self) -> bool { 
        self.has(Self::ACTIVE) 
    }

    #[inline(always)] 
    pub fn is_trusted(self) -> bool { 
        self.has(Self::TRUSTED) 
    }

    #[inline(always)] 
    pub fn is_stale(self) -> bool { 
        self.has(Self::STALE) 
    }

    #[inline(always)] 
    pub fn is_manipulation_detected(self) -> bool { 
        self.has(Self::MANIPULATION_DETECTED) 
    }

    // Conversions for account IO
    #[inline(always)] 
    pub const fn as_u8(self) -> u8 { 
        self.0 
    }

    #[inline(always)] 
    pub const fn from_u8_truncate(value: u8) -> Self {
        // lenient: drop unknown bits for forward-compat reads
        Self(value & Self::VALID_MASK)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(u8)]
pub enum SourceType {
    DEX = 0,
    CEX = 1,
    Oracle = 2,
    Aggregator = 3,
}
