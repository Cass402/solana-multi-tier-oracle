use crate::utils::constants::BUFFER_SIZE;
use anchor_lang::prelude::*;
use bytemuck::{Pod, Zeroable};

#[account(zero_copy)]
#[repr(C)]
pub struct HistoricalChunk {
    pub chunk_id: u16,
    pub head: u16,
    pub tail: u16,
    pub count: u16,
    pub creation_timestamp: u64,
    pub next_chunk: Pubkey,
    pub price_points: [PricePoint; BUFFER_SIZE],
    pub reserved: [u64; 8], // reserved for future use
}

#[derive(Clone, Copy, Debug, Pod, Zeroable)]
#[repr(C)]
pub struct PricePoint {
    pub price: i64,
    pub conf: u64,
    pub timestamp: u64,
    pub volume: u64,
    pub expo: i32,
    pub _padding: [u8; 4], // Padding for alignment
}