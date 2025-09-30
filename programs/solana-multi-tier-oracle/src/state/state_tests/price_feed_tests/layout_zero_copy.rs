use super::core_unit_tests_and_utils::sample_price_feed;
use crate::state::price_feed::{FeedFlags, PriceFeed, SourceType};
use bytemuck::{bytes_of, Pod, Zeroable};
use std::mem::{align_of, size_of};

/// Tests that document and enforce the *intentional* low-level layout
/// guarantees required by our zero-copy account handling.
///
/// Why these tests matter:
/// - On Solana, account data is stored and transferred as byte slices. To
///   avoid allocations and expensive copies we interpret those bytes directly
///   as Rust structs (zero-copy). This places a strong contract on the
///   struct's layout, alignment, and padding.
/// - Any accidental change in layout (e.g., adding a field, changing a type,
///   or reordering fields) can silently break serialization, make accounts
///   smaller/larger than expected, or introduce undefined behavior when using
///   unsafe zero-copy code. These tests act as a canary to catch such
///   regressions early during development.
#[test]
fn price_feed_struct_layout_invariants() {
    // The exact size is part of the on-chain storage contract. If this
    // assertion fails, revisit account sizing, rent calculations and any
    // client-side assumptions about byte offsets.
    assert_eq!(size_of::<PriceFeed>(), 112, "repr(C) layout changed: check account sizing");

    // `i128` fields or types that want 16-byte alignment force the overall
    // struct alignment. Misalignment can create UB when transmuting or
    // performing raw pointer casts during zero-copy reads.
    assert_eq!(align_of::<PriceFeed>(), 16, "expected 16-byte alignment due to i128 fields");

    // FeedFlags is deliberately a compact single-byte bitfield to minimize
    // account size and enable atomic updates via read-modify-write. If this
    // changes, re-evaluate bitpacking, storage cost, and all accessor helpers.
    assert_eq!(size_of::<FeedFlags>(), 1, "FeedFlags should stay a single byte bitfield");
    assert_eq!(align_of::<FeedFlags>(), 1, "FeedFlags alignment drifted—breaks zero-copy bitfield assumptions");

    // Enum discriminants must remain byte-sized; many on-chain encodings and
    // cross-program consumers assume 1-byte discriminants for compactness and
    // stable ABI. Changing this increases account size and breaks
    // cross-version compatibility.
    assert_eq!(size_of::<SourceType>(), 1, "SourceType discriminants must remain 1 byte");
    assert_eq!(align_of::<SourceType>(), 1, "SourceType alignment must stay byte-addressable");
}

/// Verify bytemuck trait constraints required for safe, constant-time
/// zero-copy casts between bytes and our structs.
///
/// Rationale:
/// - `Pod` (plain-old-data) ensures the type has no drop glue, is `repr(C)`
///   compatible, and can be safely read from raw bytes. `Zeroable` ensures a
///   valid all-zero bit pattern exists (useful for allocation/initialization
///   strategies and deterministic padding expectations).
/// - We do not rely on Rust to enforce these at compile-time across all
///   operations; an explicit test prevents accidental regressions when the
///   type graph changes.
#[test]
fn zero_copy_trait_invariants() {
    fn assert_pod<T: Pod>() {}
    fn assert_zeroable<T: Zeroable>() {}

    // If these constraints fail to hold, any zero-copy decode (e.g. via
    // `bytemuck::from_bytes`) becomes unsafe and must be re-evaluated.
    assert_pod::<PriceFeed>();
    assert_pod::<FeedFlags>();

    assert_zeroable::<PriceFeed>();
    assert_zeroable::<FeedFlags>();
}

/// Padding bytes should remain deterministically zero to avoid leaking
/// uninitialized memory or creating non-deterministic serialized blobs.
///
/// Reasons this is critical:
/// - Padding may be introduced by alignment requirements. If padding contains
///   garbage (e.g., stack or heap residues), serializing the struct will
///   include that data and break equality/merkle checks, tests, or any checksum
///   that assumes deterministic serialization.
/// - Non-zero padding can create subtle replay or storage inconsistencies when
///   accounts are snapshotted or compared across nodes.
#[test]
fn padding_bytes_remain_zero() {
    let mut feed = sample_price_feed();
    // Mutate fields to exercise write paths that might inadvertently touch
    // padding via uninitialized memory bugs in future refactors.
    feed.last_price += 1;
    feed.volume_24h += 10;
    feed.flags.set(FeedFlags::ACTIVE);

    // The in-memory `_padding` array is part of the deterministic layout and
    // must remain zero after arbitrary, legitimate updates to live fields.
    assert!(feed._padding.iter().all(|byte| *byte == 0), "padding mutated—violates deterministic layout");

    // Serialize to bytes via `bytemuck::bytes_of` and ensure the serialized
    // image does not leak non-zero values from padding. This mirrors the
    // exact scenario used by zero-copy readers that map account bytes to
    // structs directly.
    let raw = bytes_of(&feed);
    let padding_slice = &raw[raw.len() - feed._padding.len()..];
    assert!(padding_slice.iter().all(|byte| *byte == 0), "serialized padding leaked non-zero data");
}
