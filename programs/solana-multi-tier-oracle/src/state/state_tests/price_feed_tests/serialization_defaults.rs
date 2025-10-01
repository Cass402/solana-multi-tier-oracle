use super::core_unit_tests_and_utils::sample_price_feed;
use crate::state::price_feed::{FeedFlags, PriceFeed, SourceType};
use anchor_lang::prelude::Pubkey;
use anchor_lang::AnchorDeserialize;
use anchor_lang::AnchorSerialize;

/// Ensure `PriceFeed` serializes via Anchor without losing semantic state.
///
/// Why this test exists:
/// - Anchor's derive-based (de)serialization is convenient but must be
///   proven to faithfully preserve every field in our packed, zero-copy
///   oriented struct. Any divergence would break upgrades and client
///   assumptions about on-chain snapshots.
/// - The test explicitly compares important fields rather than relying on a
///   blanket `==` which makes it clear which fields are considered part of
///   the on-chain contract.
#[test]
fn price_feed_anchor_roundtrip() {
    let feed = sample_price_feed();
    let serialized = feed.try_to_vec().expect("serialize price feed");
    let deserialized = PriceFeed::try_from_slice(&serialized).expect("deserialize price feed");
    assert_eq!(feed.source_address, deserialized.source_address);
    assert_eq!(feed.last_price, deserialized.last_price);
    assert_eq!(feed.volume_24h, deserialized.volume_24h);
    assert_eq!(feed.liquidity_depth, deserialized.liquidity_depth);
    assert_eq!(feed.last_conf, deserialized.last_conf);
    assert_eq!(feed.last_update, deserialized.last_update);
    assert_eq!(feed.last_expo, deserialized.last_expo);
    assert_eq!(feed.weight, deserialized.weight);
    assert_eq!(feed.lp_concentration, deserialized.lp_concentration);
    assert_eq!(feed.manipulation_score, deserialized.manipulation_score);
    assert_eq!(feed.source_type, deserialized.source_type);
    assert_eq!(feed.flags.as_u8(), deserialized.flags.as_u8());
    // Padding must remain deterministic (zeroed) after (de)serialization.
    assert!(deserialized._padding.iter().all(|byte| *byte == 0));
}

/// Verify flags retain configured bits under pristine serialization.
///
/// Design rationale:
/// - Flags are a compact bitfield; preserving exact bits is required for any
///   logic that performs bitwise reads or cross-program comparisons.
/// - This test gives confidence that the (de)serializer doesn't introduce
///   implicit normalization in the happy path.
#[test]
fn feed_flags_anchor_roundtrip_pristine() {
    let mut flags = FeedFlags::new();
    flags.set(FeedFlags::ACTIVE);
    flags.set(FeedFlags::TRUSTED);

    let serialized = flags.try_to_vec().expect("serialize flags");
    let recovered = FeedFlags::try_from_slice(&serialized).expect("deserialize flags");

    assert!(recovered.is_active());
    assert!(recovered.is_trusted());
    assert_eq!(recovered.as_u8(), flags.as_u8());
}

/// Sanitization step must remove unknown bits that may have been injected
/// (either via corruption or forward-compat writes from newer program
/// versions).
///
/// Security implications:
/// - Unknown bits interpreted as flags could flip behavior (e.g., marking a
///   feed active/trusted) and open attack vectors. We deliberately sanitize
///   serialized inputs to the canonical mask before use.
#[test]
fn feed_flags_truncate_after_corruption() {
    let mut flags = FeedFlags::new();
    flags.set(FeedFlags::ACTIVE);
    flags.set(FeedFlags::TRUSTED);

    let mut serialized = flags.try_to_vec().expect("serialize flags");
    // Corrupt serialized buffer with extra bits to mimic forward-compatibility scenarios.
    serialized[0] |= 0b1110_0000;

    let recovered = FeedFlags::try_from_slice(&serialized).expect("deserialize flags");
    assert!(recovered.is_active());
    assert!(recovered.is_trusted());

    // Explicitly apply truncation helper to mirror the defensive path used by
    // zero-copy readers and ensure unknown bits are not promoted to live state.
    let sanitized = FeedFlags::from_u8_truncate(recovered.as_u8());
    assert!(sanitized.is_active());
    assert!(sanitized.is_trusted());
    assert_eq!(
        sanitized.as_u8() & !FeedFlags::VALID_MASK,
        0,
        "unknown bits must be truncated"
    );
}

/// `SourceType` is part of the external contract; keeping it byte-sized
/// minimizes account cost and simplifies cross-program ABI expectations.
#[test]
fn source_type_anchor_roundtrip() {
    let variants = [
        SourceType::DEX,
        SourceType::CEX,
        SourceType::Oracle,
        SourceType::Aggregator,
    ];

    for variant in variants {
        let serialized = variant.try_to_vec().expect("serialize source type");
        // One-byte discriminants keep encodings compact and predictable.
        assert_eq!(
            serialized.len(),
            1,
            "source type should occupy exactly one byte"
        );
        let recovered = SourceType::try_from_slice(&serialized).expect("deserialize source type");
        assert_eq!(variant, recovered);
    }
}

/// Defaults must provide a safe, deterministic baseline for newly
/// initialized accounts. Zeroed or conservative defaults reduce attack
/// surface during warm-up or partial initialization phases.
#[test]
fn price_feed_default_baseline() {
    let default_feed = PriceFeed::default();
    assert_eq!(default_feed.source_address, Pubkey::default());
    assert_eq!(default_feed.last_price, 0);
    assert_eq!(default_feed.volume_24h, 0);
    assert_eq!(default_feed.liquidity_depth, 0);
    assert_eq!(default_feed.last_conf, 0);
    assert_eq!(default_feed.last_update, 0);
    assert_eq!(default_feed.last_expo, 0);
    assert_eq!(default_feed.weight, 0);
    assert_eq!(default_feed.lp_concentration, 0);
    assert_eq!(default_feed.manipulation_score, 0);
    // Default to the conservative `DEX` source to avoid overly trusting
    // external or privileged feeds by default.
    assert_eq!(
        default_feed.source_type,
        SourceType::DEX.as_u8(),
        "default source should map to conservative fallback"
    );
    assert_eq!(default_feed.flags.as_u8(), 0);
    assert!(
        default_feed._padding.iter().all(|byte| *byte == 0),
        "default padding must be zeroed"
    );
}

/// Mutations to live fields must not touch padding so serialized blobs
/// remain deterministic for equality checks and hashing.
#[test]
fn padding_stays_zero_after_mutations() {
    let mut feed = PriceFeed::default();
    feed.last_price = 123;
    feed.flags.set(FeedFlags::ACTIVE);
    feed.set_source_type(SourceType::Oracle);

    // In-memory pad must remain zero after valid mutations. If this fails it
    // often indicates a future refactor that introduced an uninitialized
    // field or incorrect transmute semantics.
    assert!(
        feed._padding.iter().all(|byte| *byte == 0),
        "mutations must not alter padding"
    );

    // Round-trip serialized image should also keep padding zero to avoid
    // leaking non-deterministic data into on-chain storage.
    let serialized = feed.try_to_vec().expect("serialize mutated feed");
    let recovered = PriceFeed::try_from_slice(&serialized).expect("deserialize mutated feed");
    assert!(
        recovered._padding.iter().all(|byte| *byte == 0),
        "roundtrip must keep padding zeroed"
    );
}
