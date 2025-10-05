//! Serialization and integration smoke-tests for governance state.
//!
//! Why these tests exist
//! - Governance state is a critical shared resource used across multiple
//!   instructions in the oracle program. These tests validate that zero-copy
//!   serialization/deserialization works correctly and that permission checks
//!   are consistently enforced whether called directly on `GovernanceState` or
//!   through higher-level helpers like `OracleState::check_permission`.
//!
//! Design and safety considerations
//! - Zero-copy patterns are used on Solana to avoid heap allocations and reduce
//!   compute costs, but they require strict guarantees about memory layout,
//!   padding, and ABI stability. These tests ensure that in-place mutations
//!   (e.g., via `&mut *(bytes.as_mut_ptr() as *mut GovernanceState)`) preserve
//!   semantics and don't introduce corruption.
//! - Integration tests simulate real instruction flows (e.g., a mock
//!   `register_price_feed`) to catch mismatches between governance primitives
//!   and their usage in production code. This prevents subtle bugs where
//!   permission checks work in isolation but fail in context.
//!
//! Trade-offs and optimizations
//! - Tests use raw byte manipulation to emulate on-chain account storage,
//!   avoiding Anchor's serialization overhead. This mirrors production but
//!   requires careful unsafe code with explicit safety justifications.
//! - Round-trip tests validate that mutations persist correctly, ensuring
//!   that zero-copy views write back to the underlying account data without
//!   data loss or misalignment.

use super::helpers::{
    active_member_slice, assert_reserved_padding, assert_state_error, governance_fixture,
    governance_from_bytes, governance_to_bytes,
};
use crate::error::StateError;
use crate::state::governance_state::{GovernanceState, Permissions};
use crate::state::oracle_state::OracleState;
use anchor_lang::prelude::Pubkey;
use anchor_lang::Result as AnchorResult;

#[test]
fn zero_copy_bytes_roundtrip_preserves_semantics() {
    let state = governance_fixture(4);
    let bytes = governance_to_bytes(&state);
    let reconstructed = governance_from_bytes(&bytes);

    assert_eq!(state.proposal_threshold, reconstructed.proposal_threshold);
    assert_eq!(state.voting_period, reconstructed.voting_period);
    assert_eq!(state.execution_delay, reconstructed.execution_delay);
    assert_eq!(state.timelock_duration, reconstructed.timelock_duration);
    assert_eq!(state.veto_period, reconstructed.veto_period);
    assert_eq!(state.quorum_threshold, reconstructed.quorum_threshold);
    assert_eq!(state.multi_sig_threshold, reconstructed.multi_sig_threshold);
    assert_eq!(state.active_member_count, reconstructed.active_member_count);
    assert_eq!(
        state.allowed_dex_program_count,
        reconstructed.allowed_dex_program_count
    );
    assert_eq!(
        &state.allowed_dex_programs[..state.allowed_dex_program_count as usize],
        &reconstructed.allowed_dex_programs[..reconstructed.allowed_dex_program_count as usize]
    );
    assert_eq!(state.oracle_state, reconstructed.oracle_state);

    for (lhs, rhs) in state
        .member_permissions
        .iter()
        .zip(reconstructed.member_permissions.iter())
        .take(state.active_member_count as usize)
    {
        assert_eq!(lhs.as_u64(), rhs.as_u64());
    }

    assert_reserved_padding(&reconstructed);
}

#[test]
fn zero_copy_mutations_write_back_into_underlying_bytes() {
    let state = governance_fixture(2);
    let mut bytes = governance_to_bytes(&state);

    {
        // Simulate an instruction mutating the zero-copy account in-place.
        // This tests that mutations via a zero-copy view persist back to the
        // underlying byte buffer, which is critical for on-chain account updates
        // where data must be modified without reallocating or copying.
        let mutable_state = unsafe {
            // Safety: bytes were produced from a valid GovernanceState image,
            // so casting back preserves layout and alignment. This is unsafe
            // but mirrors production zero-copy usage; in tests, we control the
            // input to avoid undefined behavior.
            &mut *(bytes.as_mut_ptr() as *mut GovernanceState)
        };
        mutable_state
            .grant_member_permission(1, Permissions::EMERGENCY_HALT)
            .expect("grant via zero-copy view");
        mutable_state.voting_period = 12 * 60 * 60;
    }

    let reread = governance_from_bytes(&bytes);
    assert!(reread.member_permissions[1].has(Permissions::EMERGENCY_HALT));
    assert_eq!(reread.voting_period, 12 * 60 * 60);
}

#[derive(Default)]
struct MockRegisterPriceFeed;

impl MockRegisterPriceFeed {
    fn execute(governance: &GovernanceState, authority: &Pubkey) -> AnchorResult<()> {
        // Mirror the production logic inside `register_price_feed` where ADD_FEED
        // permission gates the privileged operation. This mock ensures that
        // governance checks are consistently enforced across different code paths,
        // preventing discrepancies where direct calls succeed but instruction
        // wrappers fail due to mismatched permission logic.
        governance.check_member_permission(authority, Permissions::ADD_FEED)
    }
}

#[test]
fn mocked_instruction_respects_governance_permissions() {
    let mut state = governance_fixture(3);
    state
        .grant_member_permission(0, Permissions::ADD_FEED)
        .expect("fixture member gains ADD_FEED");
    let authority = active_member_slice(&state)[0];

    MockRegisterPriceFeed::execute(&state, &authority).expect("member with ADD_FEED should pass");

    let outsider = Pubkey::new_unique();
    assert_state_error(
        MockRegisterPriceFeed::execute(&state, &outsider),
        StateError::UnauthorizedCaller,
    );
}

#[test]
fn oracle_state_permission_delegation_matches_governance() {
    let state = governance_fixture(2);
    let authority = active_member_slice(&state)[0];

    assert!(OracleState::check_permission(&state, &authority, Permissions::UPDATE_PRICE).is_ok());

    let intruder = Pubkey::new_unique();
    assert_state_error(
        OracleState::check_permission(&state, &intruder, Permissions::UPDATE_PRICE),
        StateError::UnauthorizedCaller,
    );

    let lacking_member = active_member_slice(&state)[1];
    assert_state_error(
        OracleState::check_permission(&state, &lacking_member, Permissions::EMERGENCY_HALT),
        StateError::InsufficientPermissions,
    );
}
