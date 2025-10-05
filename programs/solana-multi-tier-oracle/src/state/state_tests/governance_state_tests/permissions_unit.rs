//! Focused unit tests for the `Permissions` compact bitfield.
//!
//! Why these tests exist
//! - Permissions are represented as a compact u64-backed bitfield to minimise
//!   account size and runtime cost on Solana. That efficiency comes with the
//!   risk that subtle bitwise bugs (shifts, masking errors, or accidental
//!   clearing) can grant or remove privileges silently. These tests encode
//!   the intended bit-level semantics so any regression is obvious and
//!   intentional migration work is required for changes.
//!
//! Design and safety notes
//! - We prefer operations that are additive and reversible: `grant` should be
//!   additive, `revoke` should only clear target bits, and `toggle` should
//!   only flip the requested capability. Tests assert these properties.
//! - `VALID_MASK` exists to constrain valid bits; tests assert the mask covers
//!   declared permissions to prevent accidental introduction of unknown bits.
//! - Role helper methods (e.g., `is_admin`, `is_operator`) are composition
//!   utilities — tests validate they reflect documented masks rather than
//!   being magic checks.

use super::helpers::{assert_permissions_sanitized, PERMISSION_VARIANTS};
use crate::state::governance_state::Permissions;

#[test]
fn grant_preserves_existing_bits() {
    let mut perms = Permissions::new();
    perms.grant(Permissions::UPDATE_PRICE);

    assert!(perms.can_update_price());
    assert!(!perms.can_trigger_circuit_breaker());
    assert!(!perms.can_modify_config());

    // Granting an additional capability should layer on top without clearing
    // previously set permissions.
    perms.grant(Permissions::TRIGGER_CIRCUIT_BREAKER);
    assert!(perms.can_trigger_circuit_breaker());
    assert!(perms.can_update_price());

    assert_permissions_sanitized(perms);
}

#[test]
fn revoke_only_clears_target_bits() {
    let mut perms = Permissions::ADMIN_ALL;
    perms.revoke(Permissions::EMERGENCY_HALT);

    assert!(!perms.can_emergency_halt());
    assert!(perms.can_update_price());
    assert!(perms.can_add_feed());
    assert!(perms.can_remove_feed());

    assert_permissions_sanitized(perms);
}

#[test]
fn toggle_flips_specific_capability() {
    let mut perms = Permissions::OPERATOR_ALL;
    assert!(perms.can_view_metrics());

    perms.toggle(Permissions::VIEW_METRICS);
    assert!(!perms.can_view_metrics());
    assert!(perms.can_update_price());

    perms.toggle(Permissions::VIEW_METRICS);
    assert!(perms.can_view_metrics());

    assert_permissions_sanitized(perms);
}

#[test]
fn set_to_matches_boolean_condition() {
    let mut perms = Permissions::new();
    perms.set_to(Permissions::ADD_FEED, true);
    assert!(perms.can_add_feed());

    perms.set_to(Permissions::ADD_FEED, false);
    assert!(!perms.can_add_feed());

    perms.set_to(Permissions::REMOVE_FEED, true);
    assert!(perms.can_remove_feed());
    perms.set_to(Permissions::REMOVE_FEED, false);
    assert!(!perms.can_remove_feed());

    assert_permissions_sanitized(perms);
}

#[test]
fn role_helpers_reflect_documented_masks() {
    // ADMIN role intentionally excludes VIEW_METRICS to emphasise composition.
    let admin = Permissions::ADMIN_ALL;
    assert!(admin.is_admin());
    assert!(!admin.can_view_metrics());

    let admin_with_monitoring = Permissions::with_permissions(admin, Permissions::VIEW_METRICS);
    assert!(admin_with_monitoring.is_admin());
    assert!(admin_with_monitoring.can_view_metrics());

    let restricted =
        Permissions::without_permissions(admin_with_monitoring, Permissions::EMERGENCY_HALT);
    assert!(!restricted.is_admin());
    assert!(!restricted.can_emergency_halt());
    assert!(restricted.can_update_price());
}

#[test]
fn operator_role_requires_monitoring_and_update_bits() {
    let mut operator = Permissions::new();
    operator.grant(Permissions::UPDATE_PRICE);
    assert!(
        !operator.is_operator(),
        "missing monitoring capability must fail"
    );

    operator.grant(Permissions::VIEW_METRICS);
    assert!(operator.is_operator());

    operator.revoke(Permissions::UPDATE_PRICE);
    assert!(!operator.is_operator());
}

#[test]
fn valid_mask_covers_all_declared_permissions() {
    for perm in PERMISSION_VARIANTS {
        assert!(
            Permissions::VALID_MASK & perm.as_u64() == perm.as_u64(),
            "permission {perm:?} must remain part of VALID_MASK"
        );
    }
}
