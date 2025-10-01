//! Targeted unit tests that exercise the hot-path behaviours of the
//! `HistoricalChunk` circular buffer. Each test documents a specific safety or
//! performance guarantee relied upon by production code.

use super::helpers::{
    alternating_extreme_point, assert_chunk_invariants, assert_price_point_eq, collect_fifo_view,
    deterministic_price_point, empty_chunk, BUFFER_SIZE_U16,
};
use crate::utils::constants::BUFFER_SIZE;
use anchor_lang::prelude::Pubkey;

/// Validates that the first write into an empty buffer sets the head pointer,
/// bumps the element count, and leaves the tail parked at zero. This mirrors the
/// production bootstrapping path when a fresh historical chunk account is created.
#[test]
fn push_into_empty_chunk_updates_state() {
    let mut chunk = empty_chunk();
    let point = deterministic_price_point(0);

    chunk.push(point);

    assert_eq!(
        chunk.count, 1,
        "count should reflect the single inserted element"
    );
    assert_eq!(chunk.head, 1, "head advances to the next write position");
    assert_eq!(
        chunk.tail, 0,
        "tail remains at origin until buffer saturation"
    );
    assert_price_point_eq(&chunk.price_points[0], &point);
    assert_chunk_invariants!(chunk);
}

/// Exercises the saturation path where the buffer transitions from partially
/// filled to full and must start advancing the tail pointer to maintain FIFO
/// semantics. Anchors the expectation that once `count` reaches capacity it
/// never exceeds `BUFFER_SIZE`.
#[test]
fn push_rolls_tail_only_after_saturation() {
    let mut chunk = empty_chunk();

    for idx in 0..(BUFFER_SIZE as i64) {
        chunk.push(deterministic_price_point(idx));
    }

    assert_eq!(
        chunk.count, BUFFER_SIZE_U16,
        "buffer should report full capacity"
    );
    assert_eq!(
        chunk.head, 0,
        "head wraps back to zero after exact capacity writes"
    );
    assert_eq!(
        chunk.tail, 0,
        "tail should remain parked until the first overwrite"
    );

    let tail_before = chunk.tail;
    chunk.push(deterministic_price_point(10_000));
    assert_eq!(
        chunk.count, BUFFER_SIZE_U16,
        "count stays capped after saturation"
    );
    assert_eq!(
        chunk.tail,
        (tail_before + 1) & (BUFFER_SIZE_U16 - 1),
        "tail only advances once buffer is full"
    );
    assert_chunk_invariants!(chunk);
}

/// Confirms that repeated wraparound writes keep FIFO ordering intact. The test
/// pushes three buffer lengths worth of data and verifies we retain the most
/// recent `BUFFER_SIZE` entries in the correct logical order.
#[test]
fn sustained_wraparound_preserves_fifo_order() {
    let mut chunk = empty_chunk();
    let total_writes = BUFFER_SIZE * 3;

    let mut expected_tail = Vec::with_capacity(BUFFER_SIZE);
    for idx in 0..total_writes {
        let offset = idx as i64;
        let point = deterministic_price_point(offset);
        chunk.push(point);

        if idx >= total_writes - BUFFER_SIZE {
            expected_tail.push(point);
        }
    }

    assert_eq!(
        chunk.count, BUFFER_SIZE_U16,
        "buffer should remain saturated after long run"
    );
    let fifo_view = collect_fifo_view(&chunk);
    assert_eq!(
        fifo_view.len(),
        BUFFER_SIZE,
        "logical view must expose full capacity"
    );

    for (actual, expected) in fifo_view.iter().zip(expected_tail.iter()) {
        assert_price_point_eq(actual, expected);
    }
    assert_chunk_invariants!(chunk);
}

/// Ensures the helper accessor returns `None` when the buffer is empty, which
/// prevents callers from accidentally dereferencing stale memory.
#[test]
fn latest_returns_none_when_empty() {
    let chunk = empty_chunk();
    assert!(
        chunk.latest().is_none(),
        "empty buffers must not expose stale references"
    );
}

/// Verifies that `latest()` returns a reference to the most recently written
/// datum, even after multiple wraparounds. The reference semantics are critical
/// for zero-copy consumers that operate directly on the account backing slice.
#[test]
fn latest_tracks_last_inserted_element() {
    let mut chunk = empty_chunk();

    for idx in 0..(BUFFER_SIZE as i64 + 5) {
        chunk.push(deterministic_price_point(idx));
    }

    let latest = chunk.latest().expect("buffer should contain data");
    let expected = deterministic_price_point(BUFFER_SIZE as i64 + 4);
    assert_price_point_eq(latest, &expected);
    assert_chunk_invariants!(chunk);
}

/// Documents the `next_chunk` pointer contract: the default pubkey (all zeros)
/// acts as the sentinel meaning "end of chain".
#[test]
fn has_next_reports_chain_membership() {
    let mut chunk = empty_chunk();
    assert!(
        !chunk.has_next(),
        "default-initialised chunk must report no successor"
    );

    let next = Pubkey::new_unique();
    chunk.next_chunk = next;
    assert!(chunk.has_next(), "non-default key advertises linked chunk");

    // Regress to default to ensure the check is purely bytemuck-comparable and
    // not reliant on pointer identity.
    chunk.next_chunk = Pubkey::default();
    assert!(
        !chunk.has_next(),
        "resetting to default removes the linkage"
    );
}

/// Validates that the empty/full sentinel property holds across a mixture of
/// operations. This guards the core invariant relied upon by callers when they
/// need to distinguish the two cases where `head == tail`.
#[test]
fn tail_equals_head_only_when_empty_or_full() {
    let mut chunk = empty_chunk();

    assert_eq!(chunk.head, chunk.tail);
    assert_eq!(chunk.count, 0);

    chunk.push(deterministic_price_point(1));
    assert_ne!(
        chunk.head, chunk.tail,
        "non-empty, non-full buffers must keep head and tail distinct"
    );

    for idx in 2..=(BUFFER_SIZE as i64) {
        chunk.push(deterministic_price_point(idx));
    }
    assert_eq!(chunk.count, BUFFER_SIZE_U16);
    assert_eq!(
        chunk.head, chunk.tail,
        "full buffer re-aligns head and tail"
    );
    assert_chunk_invariants!(chunk);
}

/// Stress test that alternates between signed extremes to flush out overflow
/// regressions in arithmetic that downstream TWAP logic depends on.
#[test]
fn alternating_extremes_do_not_corrupt_buffer() {
    let mut chunk = empty_chunk();
    let total_writes = BUFFER_SIZE * 4;

    for idx in 0..total_writes {
        chunk.push(alternating_extreme_point(idx));
    }

    assert_eq!(chunk.count, BUFFER_SIZE_U16);
    let fifo_view = collect_fifo_view(&chunk);
    assert_eq!(fifo_view.len(), BUFFER_SIZE);
    assert!(
        fifo_view.iter().any(|p| p.price == i128::MAX),
        "expect to retain high watermark"
    );
    assert!(
        fifo_view.iter().any(|p| p.price == i128::MIN + 1),
        "expect to retain low watermark"
    );
    assert_chunk_invariants!(chunk);
}
