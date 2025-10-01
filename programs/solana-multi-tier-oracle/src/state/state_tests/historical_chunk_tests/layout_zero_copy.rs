//! Byte-level layout assertions that protect the zero-copy contract relied upon
//! by the on-chain program and any off-chain indexers.

use super::helpers::{
    assert_price_point_eq, chunk_from_bytes, chunk_to_bytes, deterministic_price_point,
    empty_chunk, BUFFER_SIZE_U16,
};
use crate::state::historical_chunk::{HistoricalChunk, PricePoint};
use crate::utils::constants::BUFFER_SIZE;
use anchor_lang::ZeroCopy;
use bytemuck::{bytes_of, Pod, Zeroable};
use std::mem::{align_of, size_of};

/// Enforces the documented size/alignment contract for `HistoricalChunk`. Any
/// change here must be accompanied by migrations and rent recalculations.
#[test]
fn historical_chunk_layout_contract() {
    const EXPECTED_PRICE_POINT_SIZE: usize = 48;
    assert_eq!(
        size_of::<PricePoint>(),
        EXPECTED_PRICE_POINT_SIZE,
        "PricePoint layout drifted; re-evaluate serialization contracts"
    );
    assert_eq!(
        align_of::<PricePoint>(),
        16,
        "PricePoint alignment must remain 16 bytes due to i128 fields"
    );

    const EXPECTED_CHUNK_SIZE: usize = 2 + 2 + 2 + 2 // chunk_id, head, tail, count
        + 8 // creation_timestamp
        + 32 // next_chunk
        + 32 // oracle_state
        + EXPECTED_PRICE_POINT_SIZE * BUFFER_SIZE
        + 1 // bump
        + 511; // reserved padding

    assert_eq!(
        size_of::<HistoricalChunk>(),
        EXPECTED_CHUNK_SIZE,
        "HistoricalChunk size drifted; update account sizing calculators"
    );
    assert_eq!(
        align_of::<HistoricalChunk>(),
        16,
        "HistoricalChunk alignment driven by PricePoint i128 fields"
    );
}

/// Validates zero-copy trait contracts that allow safe casts from account bytes
/// without heap allocations.
#[test]
fn zero_copy_trait_contracts() {
    fn assert_pod<T: Pod>() {}
    fn assert_zeroable<T: Zeroable>() {}
    fn assert_zero_copy<T: ZeroCopy>() {}

    assert_pod::<PricePoint>();
    assert_zeroable::<PricePoint>();
    assert_zero_copy::<HistoricalChunk>();
}

/// Padding and reserved bytes must remain zero to avoid leaking uninitialised
/// memory into serialized account images.
#[test]
fn padding_and_reserved_bytes_remain_zero() {
    let mut chunk = empty_chunk();
    for idx in 0..(BUFFER_SIZE as i64) {
        chunk.push(deterministic_price_point(idx));
    }

    assert!(
        chunk.reserved.iter().all(|byte| *byte == 0),
        "reserved padding mutated unexpectedly"
    );

    let serialized = chunk_to_bytes(&chunk);
    let padding_slice = &serialized[serialized.len() - chunk.reserved.len()..];
    assert!(
        padding_slice.iter().all(|byte| *byte == 0),
        "serialized image leaked non-zero padding"
    );

    let reread = chunk_from_bytes(&serialized);
    assert_eq!(reread.count, BUFFER_SIZE_U16);
    assert_eq!(reread.head, chunk.head);
    assert_eq!(reread.tail, chunk.tail);
    assert_price_point_eq(&reread.price_points[0], &chunk.price_points[0]);
    assert!(
        reread.reserved.iter().all(|byte| *byte == 0),
        "padding should deserialize back to zeros"
    );
}

/// Historical chunks and price points rely on zero defaulting; ensure the
/// derived implementations continue to zero every field.
#[test]
fn default_values_are_zeroed() {
    let point = PricePoint::default();
    assert_eq!(point.price, 0);
    assert_eq!(point.volume, 0);
    assert_eq!(point.conf, 0);
    assert_eq!(point.timestamp, 0);
    assert!(
        bytes_of(&point).iter().all(|byte| *byte == 0),
        "PricePoint default must be fully zeroed"
    );

    let chunk = empty_chunk();
    assert_eq!(chunk.count, 0);
    assert_eq!(chunk.head, 0);
    assert_eq!(chunk.tail, 0);
    assert!(chunk.price_points.iter().all(|p| p.timestamp == 0));
    assert!(chunk.reserved.iter().all(|byte| *byte == 0));
}
