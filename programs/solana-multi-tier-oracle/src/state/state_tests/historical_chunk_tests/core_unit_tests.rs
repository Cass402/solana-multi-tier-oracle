//! Targeted unit tests that exercise the hot-path behaviours of the
//! `HistoricalChunk` circular buffer. These tests focus on *why* the buffer
//! must behave deterministically rather than simply *what* it does. They
//! capture design decisions, invariants relied upon by higher-level logic
//! (like TWAP), and safety checks that prevent subtle corruption or
//! non-determinism in on-chain state.

use super::helpers::{
    alternating_extreme_point, assert_chunk_invariants, assert_price_point_eq, collect_fifo_view,
    deterministic_price_point, empty_chunk, BUFFER_SIZE_U16,
};
use crate::utils::constants::BUFFER_SIZE;
use anchor_lang::prelude::Pubkey;

/// Ensure bootstrapping behavior is predictable when a freshly allocated
/// `HistoricalChunk` begins receiving writes.
///
/// Design rationale:
/// - The first write must consistently initialize `head` and `count` so callers
///   that read the 'latest' element or compute FIFO windows can rely on a
///   single canonical state transition from empty -> non-empty.
/// - Keeping `tail` at zero until saturation is an intentional choice that
///   simplifies the empty/full sentinel logic (`head == tail`) used elsewhere.
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

/// Verify saturation semantics and the invariant that `count` is always
/// capped by the buffer capacity.
///
/// Why this matters:
/// - Once the circular buffer reaches capacity it should behave as a rolling
///   window: writes drop the oldest entries and `count` must not grow beyond
///   `BUFFER_SIZE`. Many consumers depend on the guarantee that `count`
///   represents how many valid, retrievable elements exist.
/// - The moment where `head` wraps back to zero is a sensitive boundary; this
///   test asserts tail advancement only occurs after the buffer becomes full
///   to avoid off-by-one or double-advance errors.
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

/// Stress the wraparound behavior across many cycles and confirm logical FIFO
/// ordering is preserved.
///
/// Rationale:
/// - In production the chunk may be continuously written. Correctness under
///   sustained wraparound ensures older values are evicted in the right
///   sequence and consumers reconstruct TWAP windows deterministically.
/// - The test builds an expected tail view to compare against the buffer's
///   FIFO projection rather than relying on physical indices, documenting the
///   intended abstraction boundary between physical storage and logical order.
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

/// The `latest()` accessor must return `None` for an empty buffer to prevent
/// callers (including zero-copy consumers) from dereferencing uninitialised
/// memory or assuming stale values.
///
/// Safety note:
/// - Returning `None` for empty avoids accidental UB in code that treats the
///   returned reference as live data. This check is a small but critical
///   defensive boundary for callers that mirror on-chain reads.
#[test]
fn latest_returns_none_when_empty() {
    let chunk = empty_chunk();
    assert!(
        chunk.latest().is_none(),
        "empty buffers must not expose stale references"
    );
}

/// `latest()` must consistently reference the most recently pushed element.
///
/// Why reference semantics matter:
/// - Several consumers expect a stable borrow into the underlying buffer so
///   they can read price points without allocating or copying. This test
///   exercises that contract under wraparound to ensure the returned pointer
///   remains valid and points at the logical latest entry.
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

/// `next_chunk` uses the default zeroed `Pubkey` as a sentinel to represent
/// the end of a linked chain of chunks.
///
/// Design trade-offs:
/// - Using an all-zero pubkey avoids adding an extra boolean flag and keeps
///   the struct compact. However, it requires callers to treat the default
///   key specially rather than relying on Option-like semantics.
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

/// The `head == tail` condition must only occur in exactly two logical
/// states: empty and full. Consumers rely on this sentinel to disambiguate
/// buffer state without extra storage.
///
/// Why the invariant is important:
/// - Many algorithms (e.g., building TWAP windows) use the `head`/`tail`
///   pointers to determine iteration bounds. If `head == tail` could occur in
///   other scenarios, callers would need additional metadata to safely iterate
///   the buffer, increasing on-chain storage and complexity.
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

/// Stress test using alternating extreme price points to exercise integer
/// boundary conditions and ensure no arithmetic or index overflow corrupts
/// buffer state.
///
/// Safety rationale:
/// - TWAP and other aggregation logic often perform arithmetic over i128
///   values. This test writes both the maximum and minimum representable
///   values (adjusted) to ensure the buffer and its consumers handle edge
///   cases without panic, overflow, or loss of ordering.
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
