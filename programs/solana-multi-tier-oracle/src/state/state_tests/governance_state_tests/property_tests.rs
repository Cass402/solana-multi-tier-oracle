//! Property-based tests for the governance permission bitfield.
//!
//! Why property tests are valuable here
//! - Compact bitfields are an efficient on-chain representation but are also
//!   prone to subtle, stateful failures when manipulated repeatedly (e.g.
//!   toggles, repeated grants/revokes, or truncation of unknown bits). Unit
//!   tests assert specific behaviours; property tests explore wide ranges of
//!   inputs and operation sequences to catch classes of bugs that are hard to
//!   enumerate manually.
//!
//! Coverage & trade-offs
//! - These tests focus on three core properties:
//!   1. Unknown-bit sanitisation: arbitrary u64 values must be truncated to a
//!      safe `VALID_MASK` so older deployments remain compatible with newer
//!      bit assignments.
//!   2. Grant/revoke semantics: helper methods should behave like manual
//!      bitset operations (additive/reversible, no collateral bit changes).
//!   3. Role composition: role helpers (e.g., `is_admin`) must reflect
//!      documented composite masks even under random subsets of primitives.
//!
//! Practical notes
//! - The `ProptestConfig` uses a modest case count to balance test runtime and
//!   coverage in CI. If you encounter a flake, increase cases locally or run
//!   failing input shrinking to reproduce minimal failing sequences.

use super::helpers::PERMISSION_VARIANTS;
use crate::state::governance_state::Permissions;
use proptest::prelude::*;

fn permission_strategy() -> impl Strategy<Value = Permissions> {
    prop_oneof![
        Just(Permissions::UPDATE_PRICE),
        Just(Permissions::TRIGGER_CIRCUIT_BREAKER),
        Just(Permissions::MODIFY_CONFIG),
        Just(Permissions::VIEW_METRICS),
        Just(Permissions::EMERGENCY_HALT),
        Just(Permissions::ADD_FEED),
        Just(Permissions::REMOVE_FEED),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 96, .. ProptestConfig::default() })]

    /// Any random bit pattern fed through `from_u64_truncate` must sanitise
    /// unsupported bits. This mirrors how future program versions might add
    /// bits while older deployments continue operating safely.
    #[test]
    fn unknown_bits_are_masked(random_bits in any::<u64>()) {
        let truncated = Permissions::from_u64_truncate(random_bits);
        prop_assert_eq!(truncated.as_u64() & !Permissions::VALID_MASK, 0);
    }

    /// Random sequences of grant/revoke operations must behave like a manual
    /// bitset tracked alongside the helper methods. This ensures our helpers
    /// remain purely additive/subtractive without touching unrelated bits.
    #[test]
    fn grant_and_revoke_match_manual_bitset(ops in proptest::collection::vec((any::<bool>(), permission_strategy()), 0..128)) {
        let mut perms = Permissions::new();
        let mut manual_mask: u64 = 0;

        for (grant, permission) in ops {
            if grant {
                perms.grant(permission);
                manual_mask |= permission.as_u64();
            } else {
                perms.revoke(permission);
                manual_mask &= !permission.as_u64();
            }

            prop_assert_eq!(perms.as_u64(), manual_mask);
            prop_assert_eq!(manual_mask & !Permissions::VALID_MASK, 0);
        }
    }

    /// Toggle operations interleaved with grants must not introduce phantom
    /// bits. This property simulates frantic operations during an incident
    /// response where operators may repeatedly flip permissions.
    #[test]
    fn toggles_stay_within_mask(mut perms in Just(Permissions::new()), ops in proptest::collection::vec(permission_strategy(), 0..64)) {
        for perm in ops {
            perms.toggle(perm);
            prop_assert_eq!(perms.as_u64() & !Permissions::VALID_MASK, 0);
            // Toggling twice restores the prior state; check by re-applying.
            let before = perms.as_u64();
            perms.toggle(perm);
            perms.toggle(perm);
            prop_assert_eq!(perms.as_u64(), before);
        }
    }

    /// Role helpers must remain consistent with the documented composite masks
    /// even when fed random subsets of the primitive permissions. This guards
    /// against future edits that might forget to update the role helpers.
    #[test]
    fn role_helpers_match_composite_masks(mask_bits in any::<u64>()) {
        let perms = Permissions::from_u64_truncate(mask_bits);
        let admin_expected = PERMISSION_VARIANTS.iter().fold(0u64, |acc, perm| {
            if Permissions::ADMIN_ALL.has(*perm) {
                acc | perm.as_u64()
            } else {
                acc
            }
        });
        let operator_expected = PERMISSION_VARIANTS.iter().fold(0u64, |acc, perm| {
            if Permissions::OPERATOR_ALL.has(*perm) {
                acc | perm.as_u64()
            } else {
                acc
            }
        });

        prop_assert_eq!(perms.is_admin(), perms.has_all(Permissions::ADMIN_ALL));
        prop_assert_eq!(perms.is_operator(), perms.has_all(Permissions::OPERATOR_ALL));
        prop_assert_eq!(Permissions::ADMIN_ALL.as_u64(), admin_expected);
        prop_assert_eq!(Permissions::OPERATOR_ALL.as_u64(), operator_expected);
    }
}
