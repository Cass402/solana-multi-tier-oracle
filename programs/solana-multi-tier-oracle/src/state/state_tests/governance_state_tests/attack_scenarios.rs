//! Negative-path regression tests that model realistic governance attack vectors.
//!
//! Why these tests exist
//! - Governance logic is a high-risk attack surface: mistakes here lead to
//!   irreversible on-chain loss or privilege escalation. These tests encode the
//!   security intentions of the `governance_state` API as explicit, verifiable
//!   expectations.
//!
//! Design intent and trade-offs
//! - We assert specific `StateError` variants instead of generic booleans so
//!   callers (and auditors) can reason precisely about failure modes. Returning
//!   well-defined errors helps on-chain clients handle failure deterministically
//!   and makes replay/analysis simpler during audits.
//! - Tests focus on invariants rather than implementation details. For example
//!   the canonical permission mask and the active-member set size are
//!   invariants that must never be violated regardless of how callers attempt to
//!   manipulate state (e.g., via captured keys or toggling bits).
//! - These tests intentionally avoid exercising private internals; they mimic
//!   how an attacker would interact with the public API to validate the
//!   surface-level guarantees.
//!
//! Safety considerations
//! - Ensure membership checks cannot be bypassed by configuration flags
//!   (e.g., `strict_mode_enabled`) or by shrinking the active set.
//! - Ensure permission bit operations are constrained to the canonical mask so
//!   unexpected bits cannot be introduced by arithmetic/bit-twiddling attacks.

use super::helpers::{
    active_member_slice, assert_state_error, governance_fixture, PERMISSION_VARIANTS,
};
use crate::error::StateError;
use crate::state::governance_state::Permissions;
use anchor_lang::prelude::Pubkey;

#[test]
fn unauthorized_caller_is_rejected_even_under_strict_mode() {
    let mut state = governance_fixture(3);
    state.strict_mode_enabled = 1; // ensure strict toggle has no effect on membership checks
    let outsider = Pubkey::new_unique();

    assert_state_error(
        state.check_member_permission(&outsider, Permissions::UPDATE_PRICE),
        StateError::UnauthorizedCaller,
    );
}

#[test]
fn valid_member_missing_required_permission_is_blocked() {
    let mut state = governance_fixture(3);
    let member = active_member_slice(&state)[1];

    // Ensure the member lacks EMERGENCY_HALT after explicit revocation.
    state
        .revoke_member_permission(1, Permissions::EMERGENCY_HALT)
        .expect("revoke emergency halt");

    assert_state_error(
        state.check_member_permission(&member, Permissions::EMERGENCY_HALT),
        StateError::InsufficientPermissions,
    );
}

#[test]
fn member_removed_via_active_count_cannot_retain_privileges() {
    let mut state = governance_fixture(4);
    let displaced_member = active_member_slice(&state)[3];

    state
        .set_active_member_count(2)
        .expect("shrink active set to simulate capture attempt");

    assert!(state.find_member(&displaced_member).is_none());
    assert_state_error(
        state.check_member_permission(&displaced_member, Permissions::UPDATE_PRICE),
        StateError::UnauthorizedCaller,
    );
}

#[test]
fn toggling_permissions_cannot_grant_unauthorized_bits() {
    let mut state = governance_fixture(2);
    let attacker_index = 0usize;

    // Simulate a captured key flipping permissions repeatedly.
    for (idx, perm) in PERMISSION_VARIANTS.iter().enumerate() {
        if idx % 2 == 0 {
            state
                .grant_member_permission(attacker_index, *perm)
                .expect("grant during attack simulation");
        } else {
            state.member_permissions[attacker_index].toggle(*perm);
        }
        assert_eq!(
            state.member_permissions[attacker_index].as_u64() & !Permissions::VALID_MASK,
            0,
            "toggle/grant must never surface unknown permission bits"
        );
    }

    // Revoking the bits must still succeed, proving we never produced
    // irreversible permissions outside the canonical mask.
    for perm in PERMISSION_VARIANTS {
        state
            .revoke_member_permission(attacker_index, perm)
            .expect("revoke after toggled grants");
    }
    assert_eq!(state.member_permissions[attacker_index].as_u64(), 0);
}
