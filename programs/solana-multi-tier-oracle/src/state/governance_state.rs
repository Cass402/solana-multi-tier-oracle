use crate::utils::constants::MAX_MULTISIG_MEMBERS;
use crate::error::StateError;
use anchor_lang::prelude::*;
use bytemuck::{Pod, Zeroable};

/// Multi-signature governance state with granular permission management for oracle operations.
/// 
/// # Governance Architecture Philosophy
/// 
/// This struct implements a sophisticated governance model that balances security with operational
/// efficiency for a financial oracle system. Key design principles:
/// 
/// - **Separation of Powers**: Distinct roles for different operational aspects prevent any single
///   entity from having excessive control over the oracle's behavior
/// - **Time-Based Safety**: Multiple time delays create windows for detection and response to
///   malicious governance actions before they take effect
/// - **Granular Permissions**: Bitfield-based permissions enable fine-grained access control
///   without the overhead of complex role hierarchies
/// - **Zero-Copy Performance**: Direct memory access for governance checks that occur on every
///   oracle operation, critical for maintaining low latency
/// 
/// # Security Model
/// 
/// The governance design addresses several attack vectors:
/// 
/// - **Governance Capture**: Multi-sig requirements prevent single-point-of-failure attacks
/// - **Flash Governance**: Time delays prevent rapid hostile takeovers
/// - **Permission Escalation**: Explicit permission checks for every privileged operation
/// - **Emergency Response**: Immediate halt capabilities for critical security incidents
#[account(zero_copy)]
#[derive(InitSpace)]
#[repr(C)]
pub struct GovernanceState {
    /// Minimum token threshold required to create governance proposals.
    /// Prevents spam proposals while maintaining democratic access to governance.
    pub proposal_threshold: u64,
    
    /// Duration in seconds for voting on governance proposals.
    /// Balances thorough deliberation with operational responsiveness.
    pub voting_period: i64,
    
    /// Mandatory delay between proposal approval and execution.
    /// Provides security window for detecting and responding to malicious proposals.
    pub execution_delay: i64,
    
    /// Duration proposals are locked in timelock before execution.
    /// Additional safety mechanism for high-impact governance changes.
    pub timelock_duration: i64,
    
    /// Period during which approved proposals can be vetoed.
    /// Emergency brake for preventing clearly harmful governance actions.
    pub veto_period: i64,
    
    /// Minimum percentage of votes required for proposal validity (basis points).
    /// Ensures governance decisions have sufficient community backing.
    pub quorum_threshold: u16,
    
    /// Number of multisig signatures required for proposal execution.
    /// Core security parameter preventing single-point governance failures.
    pub multi_sig_threshold: u8,
    
    /// Current number of active multisig members for efficient iteration.
    /// Avoids scanning entire member array when only subset is active.
    pub active_member_count: u8,
    
    /// PDA bump seed for deterministic governance account derivation.
    /// Cached to avoid recomputation during frequent governance checks.
    pub bump: u8,
    
    /// Explicit padding ensures deterministic struct layout.
    /// Critical for governance data integrity across deployment environments.
    pub _padding1: [u8; 3],

    /// Public key of the associated oracle state account.
    /// Facilitates cross-account integrity checks and operations.
    pub oracle_state: Pubkey,
    
    /// Fixed array of multisig member public keys.
    /// Fixed size enables predictable governance account costs and zero-copy access.
    pub multisig_members: [Pubkey; MAX_MULTISIG_MEMBERS],
    
    /// Permission bitfields for each multisig member.
    /// Parallel array structure optimizes cache locality for permission checks.
    pub member_permissions: [Permissions; MAX_MULTISIG_MEMBERS],
    
    /// Reserved space for future governance features without breaking changes.
    /// Sized to accommodate common governance extensions while maintaining rent exemption.
    pub reserved: [u64; 40],
}

/// Compact bitfield for governance permission flags with zero-copy performance.
/// 
/// # Design Rationale
/// 
/// Uses u64 to provide 64 distinct permission flags while maintaining efficient bitwise
/// operations. The transparent wrapper provides type safety while preserving zero-cost
/// abstractions critical for high-frequency governance checks.
/// 
/// The permission-based approach enables:
/// - **Granular Access Control**: Fine-grained permission assignment to roles
/// - **Efficient Bulk Operations**: Bitwise operations on multiple permissions
/// - **Role Composition**: Complex roles built from atomic permission flags  
/// - **Performance**: Single instruction permission checks vs multiple field comparisons
#[derive(Clone, Copy, Debug, PartialEq, Eq, Pod, Zeroable, Default, AnchorDeserialize, AnchorSerialize, InitSpace)]
#[repr(transparent)]
pub struct Permissions(u64);

impl Permissions {
    /// Permission flag definitions using explicit binary literals for audit transparency.
    /// Each permission represents an atomic capability that can be combined into roles.
    
    /// Grants ability to submit price updates to the oracle system.
    /// Core operational permission required for normal oracle functioning.
    pub const UPDATE_PRICE: Self = Self(0b0000_0001);
    
    /// Enables activation of circuit breaker mechanisms during market anomalies.
    /// Emergency permission to halt oracle operations when manipulation is detected.
    pub const TRIGGER_CIRCUIT_BREAKER: Self = Self(0b0000_0010);

    /// Allows modification of system configuration parameters.
    /// Administrative permission for tuning oracle behavior and thresholds.
    pub const MODIFY_CONFIG: Self = Self(0b0000_0100);

    /// Provides read-only access to system metrics and operational data.
    /// Monitoring permission that doesn't affect oracle state or operations.
    pub const VIEW_METRICS: Self = Self(0b0000_1000);

    /// Enables system-wide emergency halt for critical security incidents.
    /// Nuclear option that completely stops oracle operations until manual restart.
    pub const EMERGENCY_HALT: Self = Self(0b0001_0000);

    /// Grants ability to register new price feed sources.
    /// Administrative permission for expanding oracle's data source coverage.
    pub const ADD_FEED: Self = Self(0b0010_0000);

    /// Allows removal of existing price feed sources.
    /// Administrative permission for oracle maintenance and source quality management.
    pub const REMOVE_FEED: Self = Self(0b0100_0000);

    /// Comprehensive administrative role combining all management capabilities.
    /// Intentionally excludes VIEW_METRICS to demonstrate role composition patterns.
    /// Designed for full system administrators who need complete operational control.
    pub const ADMIN_ALL: Self = Self(
        Self::UPDATE_PRICE.0 
        | Self::TRIGGER_CIRCUIT_BREAKER.0 
        | Self::MODIFY_CONFIG.0 
        | Self::EMERGENCY_HALT.0 
        | Self::ADD_FEED.0 
        | Self::REMOVE_FEED.0
    );
    
    /// Limited operational role for routine oracle maintenance.
    /// Combines price updates with monitoring access while excluding administrative powers.
    /// Designed for operators who manage day-to-day oracle operations without full control.
    pub const OPERATOR_ALL: Self = Self(
        Self::UPDATE_PRICE.0 
        | Self::VIEW_METRICS.0
    );

    /// Validation mask for all currently recognized permission bits.
    /// Used for forward-compatible deserialization that ignores future permission additions.
    pub const VALID_MASK: u64 = Self::UPDATE_PRICE.0
        | Self::TRIGGER_CIRCUIT_BREAKER.0
        | Self::MODIFY_CONFIG.0
        | Self::VIEW_METRICS.0
        | Self::EMERGENCY_HALT.0
        | Self::ADD_FEED.0
        | Self::REMOVE_FEED.0;

    /// Creates empty permission set with no capabilities enabled.
    /// const fn enables compile-time initialization for secure default states.
    #[inline(always)]
    pub const fn new() -> Self { 
        Self(0) 
    }

    /// Tests whether any of the specified permission bits are present.
    /// Uses efficient bitwise AND for single-instruction permission verification.
    /// This is the fundamental operation underlying all permission checking logic.
    #[inline(always)]
    pub fn has(self, permission: Self) -> bool {
        (self.0 & permission.0) != 0
    }

    /// Semantic alias for `has()` with explicit "any" semantics.
    /// Improves code readability when checking for any of multiple permissions.
    /// Compiles to identical assembly as `has()` but clarifies intent in calling code.
    #[inline(always)]
    pub fn has_any(self, permission: Self) -> bool {
        self.has(permission)
    }

    /// Verifies that all specified permission bits are present.
    /// Critical for role verification where incomplete permission sets indicate
    /// privilege escalation attempts or configuration errors.
    #[inline(always)]
    pub fn has_all(self, permissions: Self) -> bool {
        (self.0 & permissions.0) == permissions.0
    }

    /// Adds new permissions using bitwise OR without affecting existing ones.
    /// Named "grant" to emphasize the additive security semantics in governance contexts.
    /// Preserves existing permissions to prevent accidental privilege reduction.
    #[inline(always)]
    pub fn grant(&mut self, permission: Self) {
        self.0 |= permission.0;
    }

    /// Removes specific permissions using bitwise AND with negation.
    /// Named "revoke" to emphasize the security-critical nature of permission removal.
    /// Carefully preserves other permissions to prevent unintended privilege loss.
    #[inline(always)]
    pub fn revoke(&mut self, permission: Self) {
        self.0 &= !permission.0;
    }

    /// Inverts specified permission bits for state-dependent permission logic.
    /// Useful for implementing permission workflows that depend on current authorization state.
    /// Use with caution as toggle semantics can create unpredictable security states.
    #[inline(always)]
    pub fn toggle(&mut self, permission: Self) {
        self.0 ^= permission.0;
    }

    /// Conditionally grants or revokes permissions based on boolean condition.
    /// Eliminates error-prone conditional logic in permission management code.
    /// Enables clean authorization state synchronization with external conditions.
    #[inline(always)]
    pub fn set_to(&mut self, permission: Self, granted: bool) {
        if granted { 
            self.grant(permission) 
        } else { 
            self.revoke(permission) 
        }
    }

    /// High-level semantic permission checkers for common oracle operations.
    /// These methods provide self-documenting APIs while compiling to identical assembly
    /// as direct bitwise operations. They improve audit readability and reduce errors
    /// from manually managing permission bit patterns.
    
    #[inline(always)]
    pub fn can_update_price(self) -> bool {
        self.has(Self::UPDATE_PRICE)
    }

    #[inline(always)]
    pub fn can_trigger_circuit_breaker(self) -> bool {
        self.has(Self::TRIGGER_CIRCUIT_BREAKER)
    }

    #[inline(always)]
    pub fn can_modify_config(self) -> bool {
        self.has(Self::MODIFY_CONFIG)
    }

    #[inline(always)]
    pub fn can_view_metrics(self) -> bool {
        self.has(Self::VIEW_METRICS)
    }

    #[inline(always)]
    pub fn can_emergency_halt(self) -> bool {
        self.has(Self::EMERGENCY_HALT)
    }

    #[inline(always)]
    pub fn can_add_feed(self) -> bool {
        self.has(Self::ADD_FEED)
    }

    #[inline(always)]
    pub fn can_remove_feed(self) -> bool {
        self.has(Self::REMOVE_FEED)
    }

    /// Verifies complete administrative role membership.
    /// Used for operations that require full administrative privileges.
    #[inline(always)]
    pub fn is_admin(self) -> bool {
        self.has_all(Self::ADMIN_ALL)
    }

    /// Verifies operational role membership with limited privileges.
    /// Used for day-to-day oracle operations that don't require administrative access.
    #[inline(always)]
    pub fn is_operator(self) -> bool {
        self.has_all(Self::OPERATOR_ALL)
    }

    /// Composes custom roles by combining base role with additional permissions.
    /// Enables flexible role creation without hardcoding every possible combination.
    /// const fn allows compile-time role composition for optimal performance.
    #[inline(always)]
    pub const fn with_permissions(base: Self, additional: Self) -> Self {
        Self(base.0 | additional.0)
    }

    /// Creates restricted roles by removing specific permissions from base role.
    /// Useful for creating specialized roles or implementing least-privilege principles.
    /// const fn enables compile-time role restriction for security-critical applications.
    #[inline(always)]
    pub const fn without_permissions(base: Self, excluded: Self) -> Self {
        Self(base.0 & !excluded.0)
    }

    /// Serialization helpers for zero-copy account data persistence.
    
    /// Extracts raw permission bits for account storage.
    /// const fn enables compile-time evaluation for static permission sets.
    #[inline(always)]
    pub const fn as_u64(self) -> u64 {
        self.0
    }

    /// Reconstructs permissions from account data with defensive validation.
    /// Masks unknown permission bits to prevent crashes when reading data from
    /// future program versions that define additional permissions. This forward-compatible
    /// approach ensures governance systems remain operational during upgrades.
    #[inline(always)]
    pub const fn from_u64_truncate(value: u64) -> Self {
        Self(value & Self::VALID_MASK)
    }
}

impl GovernanceState {
    /// Updates the number of active multisig members with comprehensive validation.
    /// # Governance Implications
    /// 
    /// Changing the active member count effectively modifies the governance structure:
    /// - **Reducing Count**: Temporarily disables members beyond the new count without removing their data
    /// - **Increasing Count**: Activates previously inactive member slots (data must be pre-populated)
    /// - **Signature Thresholds**: May affect multisig signature requirements depending on implementation
    pub fn set_active_member_count(&mut self, count: u8) -> Result<()> {
        require!(
            (count as usize) <= MAX_MULTISIG_MEMBERS,
            StateError::TooManyActiveMembers
        );

        self.active_member_count = count;

        Ok(())
    }

    /// Grants specific permissions to a multisig member with bounds checking.
    /// 
    /// # Security Design
    /// 
    /// This method implements several security safeguards:
    /// - Bounds checking prevents out-of-bounds access that could corrupt adjacent account data
    /// - Index validation against active_member_count ensures only legitimate members are modified
    /// - Explicit error return enables proper error handling in calling instruction handlers
    /// 
    /// The bounds check is critical because invalid array access in zero-copy account data
    /// could silently corrupt other fields or cause unpredictable behavior during execution.
    pub fn grant_member_permission(&mut self, member_index: usize, permission: Permissions) -> Result<()> {
        require!(
            member_index < self.active_member_count as usize,
            StateError::UnauthorizedCaller
        );
        
        self.member_permissions[member_index].grant(permission);
        Ok(())
    }

    /// Revokes specific permissions from a multisig member with security validation.
    /// 
    /// # Permission Revocation Safety
    /// 
    /// Permission revocation requires the same security guarantees as granting to prevent
    /// privilege escalation through invalid member targeting. The bounds check ensures
    /// attackers cannot corrupt governance state by targeting invalid member indices.
    pub fn revoke_member_permission(&mut self, member_index: usize, permission: Permissions) -> Result<()> {
        require!(
            member_index < self.active_member_count as usize,
            StateError::UnauthorizedCaller
        );
        
        self.member_permissions[member_index].revoke(permission);
        Ok(())
    }

    /// Retrieves permission set for a member with safe indexing.
    /// 
    /// # Return Strategy
    /// 
    /// Returns `Option<Permissions>` rather than potentially panicking to enable graceful
    /// handling of invalid member queries. This defensive approach prevents governance
    /// operations from failing due to programming errors in member index calculations.
    pub fn get_member_permissions(&self, member_index: usize) -> Option<Permissions> {
        if member_index < self.active_member_count as usize {
            Some(self.member_permissions[member_index])
        } else {
            None
        }
    }

    /// Locates a member by public key and returns their governance metadata.
    /// 
    /// # Performance Considerations
    /// 
    /// This method performs a linear scan through the member array, which is acceptable
    /// given the small, fixed size of the multisig (typically 3-10 members). The scan
    /// terminates early when a match is found, and the double-check against active_member_count
    /// prevents returning data for inactive member slots.
    /// 
    /// # Security Validation
    /// 
    /// The active member count check is essential because the member array may contain
    /// leftover data from previously removed members. Only checking indices within the
    /// active range prevents authorization based on stale member data.
    pub fn find_member(&self, member_key: &Pubkey) -> Option<(usize, Permissions)> {
        for (i, member) in self.multisig_members.iter().enumerate() {
            if member == member_key && i < self.active_member_count as usize {
                return Some((i, self.member_permissions[i]));
            }
        }
        None
    }

    /// Primary authorization gate for all governance-protected operations.
    /// 
    /// # Security Architecture
    /// 
    /// This method serves as the central authorization checkpoint for the entire oracle system.
    /// Every operation that requires governance permissions must funnel through this method,
    /// ensuring consistent security enforcement and audit trails.
    /// 
    /// # Two-Phase Authorization
    /// 
    /// 1. **Identity Verification**: Confirms the caller is a registered multisig member
    /// 2. **Permission Validation**: Verifies the member possesses required capabilities
    /// 
    /// This separation enables fine-grained permission tracking and helps identify whether
    /// authorization failures are due to unregistered callers or insufficient permissions.
    /// 
    /// # Error Semantics
    /// 
    /// Different error types enable callers to distinguish between:
    /// - `UnauthorizedCaller`: Not a multisig member (identity failure)
    /// - `InsufficientPermissions`: Valid member lacking required permissions (authorization failure)
    /// 
    /// This distinction is valuable for debugging and provides different response strategies
    /// for different failure modes.
    pub fn check_member_permission(&self, member_key: &Pubkey, required_permission: Permissions) -> Result<()> {
        if let Some((_, permissions)) = self.find_member(member_key) {
            require!(
                permissions.has(required_permission),
                StateError::InsufficientPermissions
            );
            Ok(())
        } else {
            Err(StateError::UnauthorizedCaller.into())
        }
    }
}