use crate::state::price_feed::{FeedFlags, PriceFeed, SourceType};
use anchor_lang::AnchorDeserialize;
use proptest::collection::vec;
use proptest::prelude::*;
use std::mem::size_of;

proptest! {
    /// Defensive truncation ensures only the intended compact bitfield is ever
    /// interpreted as authoritative state.
    ///
    /// Why this test exists:
    /// - The oracle uses a compact u8 bitfield for flags to minimize account
    ///   size and avoid heap allocations. That packing exposes the program to
    ///   accidental or malicious bit patterns in account data. We therefore
    ///   explicitly filter unknown bits via `VALID_MASK`.
    /// - Silently discarding unknown bits is a deliberate forward-compatibility
    ///   choice: new flags added in later program versions should not cause
    ///   older nodes to error when reading historic accounts. Tests assert the
    ///   filter is tight (no unknown bits survive) and idempotent.
    #[test]
    fn prop_feed_flags_truncate(raw in any::<u8>()) {
        // Apply the runtime filter used by zero-copy deserializers.
        let filtered = FeedFlags::from_u8_truncate(raw);

        // No bits outside the VALID_MASK should remain set. This prevents
        // unknown/corrupted bits from being interpreted as meaningful flags
        // which could change contract logic or enable attacks.
        prop_assert_eq!(filtered.as_u8() & !FeedFlags::VALID_MASK, 0);

        // Re-applying the truncation must be a no-op (idempotence). This is
        // important because code paths may re-normalize flags; idempotence
        // avoids surprising state changes.
        let filtered_again = FeedFlags::from_u8_truncate(filtered.as_u8());
        prop_assert_eq!(filtered.as_u8(), filtered_again.as_u8());
    }

    /// SourceType discriminants must be stable and reject unknown values.
    ///
    /// Rationale and trade-offs:
    /// - Enum discriminants are part of the on-chain binary contract. Other
    ///   programs or off-chain tooling may serialize/deserialize these values.
    ///   Accepting arbitrary discriminants risks undefined behavior or
    ///   cross-program incompatibility.
    /// - We prefer a conservative default (`DEX`) when encountering an
    ///   unknown discriminant via `from_u8_or_default`. Defaulting is safer
    ///   than panicking or silently mapping to an unrelated variant.
    #[test]
    fn prop_source_type_from_u8_behaviour(raw in any::<u8>()) {
        match SourceType::from_u8(raw) {
            Some(variant) => {
                // If the raw byte corresponds to a known variant the round-trip
                // must preserve the exact discriminant. This guarantees
                // cross-version compatibility for known variants.
                prop_assert_eq!(variant.as_u8(), raw);
            }
            None => {
                // Unknown discriminants should be handled via a well-defined
                // fallback to avoid surprising behavior in consumers.
                let defaulted = SourceType::from_u8_or_default(raw);
                prop_assert_eq!(defaulted, SourceType::DEX);
            }
        }
    }

    /// Semantic accessors must align with the underlying bitmask operations.
    ///
    /// Motivation:
    /// - Accessor helper methods (e.g. `is_active`) simplify call-sites but
    ///   must be guaranteed to reflect raw bit operations to avoid subtle
    ///   inconsistencies between optimized paths and canonical checks.
    /// - This is especially important when using bitwise flags to represent
    ///   multiple orthogonal states packed into a single byte.
    #[test]
    fn prop_feed_flags_semantic_accessors(raw in any::<u8>()) {
        let flags = FeedFlags::from_u8_truncate(raw);

        // Ensure higher-level boolean helpers match the canonical `has` bit
        // checks. If these diverge, callers relying on either API could see
        // different behavior which would be a critical bug.
        prop_assert_eq!(flags.is_active(), flags.has(FeedFlags::ACTIVE));
        prop_assert_eq!(flags.is_trusted(), flags.has(FeedFlags::TRUSTED));
        prop_assert_eq!(flags.is_stale(), flags.has(FeedFlags::STALE));
        prop_assert_eq!(flags.is_manipulation_detected(), flags.has(FeedFlags::MANIPULATION_DETECTED));
    }

    /// Rejecting arbitrary-length byte blobs prevents accidental UB in
    /// zero-copy deserializers and ensures only correctly-sized account data
    /// is accepted.
    ///
    /// Why we check lengths explicitly:
    /// - `PriceFeed` is a packed, zero-copy-friendly struct. Attempting to
    ///   deserialize from an incorrectly sized slice may either fail or, in
    ///   unsafe code paths, lead to misinterpretation of memory.
    /// - Tests use `size_of::<PriceFeed>()` so they automatically adapt when
    ///   the struct layout changes, avoiding brittle hard-coded sizes.
    #[test]
    fn prop_price_feed_deserialization_rejects_invalid_lengths(bytes in vec(any::<u8>(), 0..256)) {
        // Only consider inputs whose length is intentionally wrong for the
        // current `PriceFeed` layout; this assumption keeps the test focused
        // on the rejection behavior.
        prop_assume!(bytes.len() != size_of::<PriceFeed>());

        // Deserializing arbitrary blobs must return an error. This prevents
        // malformed input from being treated as a valid account snapshot.
        prop_assert!(PriceFeed::try_from_slice(&bytes).is_err());
    }
}
