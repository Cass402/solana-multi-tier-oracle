use crate::error::StateError;
use crate::state::governance_state::{GovernanceState, Permissions};
use crate::state::historical_chunk::{HistoricalChunk, PricePoint};
use crate::state::oracle_state::{OracleState, PriceData, StateFlags, Version};
use crate::utils::constants::{
    BUFFER_SIZE, DEFAULT_VETO_PERIOD, GOVERNANCE_SEED, HISTORICAL_CHUNK_SEED,
    MAX_CONFIDENCE_THRESHOLD, MAX_MANIPULATION_THRESHOLD, MAX_MULTISIG_MEMBERS,
    MAX_QUORUM_THRESHOLD, MAX_TWAP_WINDOW, ORACLE_STATE_SEED,
};
use crate::utils::events::OracleInitialized;
/// Comprehensive oracle initialization with governance integration and historical data architecture.
///
/// # Initialization Strategy
///
/// This module implements a one-time initialization process that establishes the complete
/// oracle infrastructure including price tracking, governance mechanisms, and historical
/// data storage. The design prioritizes immutability of core parameters and comprehensive
/// validation to prevent misconfiguration that could compromise oracle integrity.
///
/// # Multi-Account Architecture
///
/// Creates a network of interconnected accounts that work together to provide oracle
/// functionality while maintaining clear separation of concerns:
/// - Oracle state for price data and configuration
/// - Governance state for decentralized control mechanisms  
/// - Historical chunks for time-series price storage
///
/// This separation enables efficient zero-copy access patterns while maintaining
/// data integrity through cross-account validation.
use anchor_lang::prelude::*;
use anchor_lang::solana_program::keccak;

/// Comprehensive oracle configuration with embedded governance parameters.
///
/// # Configuration Encapsulation Strategy
///
/// Bundles all initialization parameters into a single struct to ensure atomic
/// configuration and prevent partial initialization states that could leave the
/// oracle in an inconsistent condition. This approach also enables comprehensive
/// validation of parameter relationships before any account creation begins.
///
/// # Immutability by Design
///
/// Most parameters become immutable after initialization, reflecting the principle
/// that core oracle characteristics should remain stable to maintain trust and
/// prevent governance attacks through parameter manipulation.
#[derive(AnchorSerialize, AnchorDeserialize, Clone)]
pub struct OracleConfig {
    /// Human-readable asset identifier for logging and display purposes only.
    /// Not used in any cryptographic operations to prevent injection attacks.
    /// Length limits enforced during validation to prevent resource exhaustion.
    pub asset_id: String,

    /// Cryptographic seed derived from canonical asset identifier.
    /// Ensures deterministic oracle account addresses while preventing collisions
    /// through the use of keccak hashing. This approach enables predictable
    /// account discovery while maintaining cryptographic uniqueness.
    pub asset_seed: [u8; 32],

    /// Time window for TWAP calculations in seconds.
    /// Determines the balance between price responsiveness and manipulation resistance.
    /// Longer windows provide better attack resistance but slower price adaptation.
    pub twap_window: u32,

    /// Minimum confidence threshold (basis points) for price data acceptance.
    /// Controls the quality gate for price information, with higher values
    /// requiring more stable price behavior before accepting updates.
    pub confidence_threshold: u16,

    /// Manipulation detection threshold (basis points) for circuit breaker activation.
    /// Defines the sensitivity of manipulation detection algorithms, balancing
    /// false positives against detection effectiveness.
    pub manipulation_threshold: u16,

    /// Emergency administrator with circuit breaker override capabilities.
    /// Provides fail-safe mechanism for critical situations while maintaining
    /// decentralization for normal operations. Should be a trusted multisig.
    pub emergency_admin: Pubkey,

    /// Enable automatic circuit breaker functionality.
    /// Controls whether the oracle will automatically halt updates when
    /// manipulation is detected, trading availability for security.
    pub enable_circuit_breaker: bool,

    /// Embedded governance configuration for decentralized control.
    /// Integrated into oracle config to ensure governance is established
    /// simultaneously with oracle creation, preventing governance gaps.
    pub governance_config: GovernanceConfig,
}

/// Governance system configuration with multisig and voting parameters.
///
/// # Decentralized Control Framework
///
/// Establishes a comprehensive governance system that balances decentralization
/// with operational efficiency. The design enables flexible decision-making
/// while preventing governance attacks through careful parameter validation
/// and permission management.
///
/// # Security Through Diversity
///
/// Multiple governance parameters create layers of protection against various
/// attack vectors, requiring attackers to compromise multiple mechanisms
/// simultaneously to gain control of the oracle system.
#[derive(AnchorSerialize, AnchorDeserialize, Clone)]
pub struct GovernanceConfig {
    /// Number of active governance members in the multisig system.
    /// Determines the size of the governance body and affects decentralization
    /// vs efficiency trade-offs. Larger groups provide better decentralization
    /// but may suffer from coordination challenges.
    pub member_count: u8,

    /// Fixed-size array of initial governance member public keys.
    /// Using fixed array instead of Vec avoids dynamic allocation and enables
    /// zero-copy deserialization. Unused slots are filled with default Pubkeys.
    pub initial_members: [Pubkey; MAX_MULTISIG_MEMBERS],

    /// Permission levels for each governance member.
    /// Enables granular access control where different members can have
    /// different capabilities, supporting role-based governance structures.
    pub member_permissions: [Permissions; MAX_MULTISIG_MEMBERS],

    /// Minimum signatures required for multisig operations.
    /// Core security parameter that determines the threshold for governance
    /// decisions. Must balance security (higher thresholds) with liveness
    /// (lower thresholds for continued operation).
    pub multisig_threshold: u8,

    /// Duration in seconds for proposal voting periods.
    /// Provides adequate time for member participation while preventing
    /// indefinite proposal delays that could halt governance operations.
    pub voting_period: i64,

    /// Delay in seconds between proposal approval and execution.
    /// Timelock mechanism that provides transparency and allows for
    /// emergency intervention if malicious proposals are approved.
    pub execution_delay: i64,

    /// Minimum participation threshold (basis points) for valid governance votes.
    /// Prevents minority control by requiring meaningful participation levels
    /// before governance decisions are considered legitimate.
    pub quorum_threshold: u16,

    /// Minimum stake required to create governance proposals.
    /// Economic barrier to prevent spam proposals while ensuring legitimate
    /// stakeholders can participate in governance processes.
    pub proposal_threshold: u64,
}

/// Account structure for oracle initialization with comprehensive validation requirements.
///
/// # PDA Determinism Strategy
///
/// All accounts use Program Derived Addresses (PDAs) to ensure deterministic account
/// creation and prevent address collision attacks. The seed structures create a
/// hierarchical relationship where governance and historical accounts are derived
/// from the oracle state, establishing clear ownership chains.
///
/// # Zero-Copy Account Design
///
/// Uses AccountLoader for all state accounts to enable zero-copy deserialization,
/// critical for performance when dealing with large historical data structures.
/// This approach avoids expensive memory allocations during frequent oracle operations.
///
/// # Multi-Chunk Historical Architecture
///
/// Initializes three historical chunks simultaneously to establish the circular
/// buffer system from the start. This prevents the complexity of dynamic chunk
/// allocation while ensuring adequate historical data capacity for TWAP calculations.
#[derive(Accounts)]
#[instruction(config: OracleConfig)]
pub struct InitializeOracle<'info> {
    /// Main oracle state account containing price data and configuration.
    /// Seeds include asset_seed to enable multiple oracles per program while
    /// preventing collisions through cryptographic uniqueness.
    #[account(
        init,
        payer = authority,
        space = 8 + OracleState::INIT_SPACE,
        seeds = [ORACLE_STATE_SEED, &config.asset_seed],
        bump,
    )]
    pub oracle_state: AccountLoader<'info, OracleState>,

    /// Governance state account for decentralized oracle control.
    /// Derived from oracle_state to establish clear ownership relationship
    /// and prevent governance account spoofing or misassociation.
    #[account(
        init,
        payer = authority,
        space = 8 + GovernanceState::INIT_SPACE,
        seeds = [GOVERNANCE_SEED, oracle_state.key().as_ref()],
        bump,
    )]
    pub governance_state: AccountLoader<'info, GovernanceState>,

    /// First historical chunk in the circular buffer system.
    /// Index [0] in seeds ensures this is always the initial chunk
    /// in the historical data sequence, providing predictable access patterns.
    #[account(
        init,
        payer = authority,
        space = 8 + HistoricalChunk::INIT_SPACE,
        seeds = [HISTORICAL_CHUNK_SEED, oracle_state.key().as_ref(), &[0]],
        bump,
    )]
    pub historical_chunk_0: AccountLoader<'info, HistoricalChunk>,

    /// Second historical chunk with index [1] for continued data storage.
    /// Forms part of the circular buffer that enables continuous historical
    /// data retention without requiring dynamic account management.
    #[account(
        init,
        payer = authority,
        space = 8 + HistoricalChunk::INIT_SPACE,
        seeds = [HISTORICAL_CHUNK_SEED, oracle_state.key().as_ref(), &[1]],
        bump,
    )]
    pub historical_chunk_1: AccountLoader<'info, HistoricalChunk>,

    /// Third historical chunk completing the circular buffer system.
    /// Three chunks provide adequate historical depth for meaningful TWAP
    /// calculations while maintaining manageable account rent costs.
    #[account(
        init,
        payer = authority,
        space = 8 + HistoricalChunk::INIT_SPACE,
        seeds = [HISTORICAL_CHUNK_SEED, oracle_state.key().as_ref(), &[2]],
        bump,
    )]
    pub historical_chunk_2: AccountLoader<'info, HistoricalChunk>,

    /// Authority account responsible for paying initialization costs and
    /// establishing initial governance membership. Must be included in the
    /// governance member list with administrative permissions.
    #[account(mut)]
    pub authority: Signer<'info>,

    /// System program required for account creation operations.
    pub system_program: Program<'info, System>,
}

/// Normalize asset identifier to canonical form for consistent seed generation.
///
/// # Canonicalization Strategy
///
/// Converts asset identifiers to lowercase and trims whitespace to prevent
/// case sensitivity issues and accidental padding that could create different
/// oracle instances for the same logical asset. This standardization ensures
/// that "SOL/USDC", "sol/usdc", and " SOL/USDC " all resolve to the same oracle.
///
/// # Attack Prevention
///
/// Prevents oracle duplication attacks where malicious actors could create
/// multiple oracles for the same asset using slight variations in capitalization
/// or whitespace, potentially splitting liquidity or confusing price consumers.
#[inline(always)]
fn canonicalize_asset_id(asset_id: &str) -> String {
    asset_id.trim().to_ascii_lowercase()
}

/// Validate asset seed matches the canonical asset identifier through cryptographic verification.
///
/// # Cryptographic Integrity
///
/// Uses Keccak hashing to ensure the provided asset_seed was legitimately derived
/// from the canonical asset identifier, preventing seed manipulation attacks where
/// malicious actors could create oracles with misleading asset_ids but valid seeds.
///
/// # Deterministic Address Generation
///
/// This validation ensures that oracle addresses are deterministic based on asset
/// identifiers, enabling predictable oracle discovery while maintaining cryptographic
/// security against collision or preimage attacks.
#[inline(always)]
fn validate_asset_seed(canonical_asset_id: &str, asset_seed: &[u8; 32]) -> Result<()> {
    let expected_hash = keccak::hashv(&[canonical_asset_id.as_bytes()]).0;

    require!(expected_hash == *asset_seed, StateError::InvalidAssetSeed);

    Ok(())
}

/// Validate asset identifier format and length constraints.
///
/// # Resource Protection Strategy
///
/// Enforces reasonable length limits to prevent resource exhaustion attacks through
/// extremely long asset identifiers that could consume excessive memory or storage.
/// The 64-character limit provides ample space for legitimate asset pairs while
/// preventing abuse.
///
/// # Data Integrity Requirements
///
/// Ensures asset identifiers are non-empty to prevent creation of oracles without
/// clear asset association, which could lead to confusion or misuse in downstream
/// applications that rely on asset identification.
#[inline(always)]
fn validate_asset_id(canonical_asset_id: &str) -> Result<()> {
    require!(
        !canonical_asset_id.is_empty() && canonical_asset_id.len() <= 64,
        StateError::InvalidAssetId
    );

    Ok(())
}

/// Comprehensive validation of governance member configuration and authority permissions.
///
/// # Governance Security Framework
///
/// Implements multiple validation layers to ensure proper governance establishment:
/// 1. Member uniqueness to prevent vote duplication
/// 2. Authority inclusion with admin rights to prevent lockout
/// 3. Valid member keys to prevent governance gaps
///
/// # Authority Bootstrap Security
///
/// Requires the initializing authority to be included as an admin member, preventing
/// scenarios where governance is established without any administrative access.
/// This ensures there's always a path for legitimate governance operations.
///
/// # Attack Vector Prevention
///
/// Prevents several governance attacks:
/// - Duplicate member attacks (same key counted multiple times)
/// - Authority lockout (initializer excluded from governance)
/// - Invalid member injection (default/null keys in governance)
#[inline(always)]
fn validate_initial_members_and_authority_admin(
    initial_members: &[Pubkey; MAX_MULTISIG_MEMBERS],
    member_count: u8,
    authority: &Pubkey,
    member_permissions: &[Permissions; MAX_MULTISIG_MEMBERS],
) -> Result<()> {
    let mut admin_authority = false;

    // Validate each active member and check for authority inclusion
    for i in 0..member_count as usize {
        // Prevent governance gaps through invalid member keys
        require!(
            initial_members[i] != Pubkey::default(),
            StateError::InvalidMemberKey
        );

        // Verify authority is included with administrative permissions
        if initial_members[i] == *authority {
            require!(
                member_permissions[i].is_admin(),
                StateError::AuthorityNotAdminMember
            );
            admin_authority = true;
        }

        // Prevent vote duplication through member uniqueness validation
        for j in (i + 1)..member_count as usize {
            require!(
                initial_members[i] != initial_members[j],
                StateError::DuplicateMember
            );
        }
    }

    // Ensure authority has administrative access to prevent governance lockout
    require!(admin_authority, StateError::AuthorityNotAdminMember);

    Ok(())
}

/// Orchestrate comprehensive oracle system initialization with full validation.
///
/// # Atomic Initialization Strategy
///
/// Performs all validation before any account modifications to ensure either
/// complete success or complete failure, preventing partial initialization states
/// that could leave the oracle system in an inconsistent or vulnerable condition.
///
/// # Security-First Validation Pipeline
///
/// Implements comprehensive validation in phases:
/// 1. Asset identifier and seed validation
/// 2. Oracle parameter bounds checking  
/// 3. Governance configuration validation
/// 4. Cross-account relationship establishment
///
/// # Account Relationship Architecture
///
/// Establishes a complex web of account relationships that enable the oracle
/// to function as a cohesive system while maintaining clear separation of
/// concerns for security and maintainability.
pub fn initialize_oracle(ctx: Context<InitializeOracle>, config: OracleConfig) -> Result<()> {
    let timestamp_now = Clock::get()?.unix_timestamp;

    // Phase 1: Asset Identifier Validation and Canonicalization
    // Ensures consistent asset identification across the ecosystem
    let canonical_asset_id = canonicalize_asset_id(&config.asset_id);

    validate_asset_id(&canonical_asset_id)?;
    validate_asset_seed(&canonical_asset_id, &config.asset_seed)?;

    // Phase 2: Oracle Parameter Validation
    // Validates all oracle configuration parameters are within safe operational bounds

    // TWAP window validation - must be positive and within system limits
    // Zero window would make TWAP calculation meaningless, excessive windows could enable stale data attacks
    require!(
        config.twap_window > 0 && config.twap_window <= MAX_TWAP_WINDOW,
        StateError::InvalidTWAPWindow
    );

    // Confidence threshold validation - controls quality gate for price acceptance
    // Higher values require more stable price behavior before accepting updates
    require!(
        config.confidence_threshold <= MAX_CONFIDENCE_THRESHOLD,
        StateError::InvalidConfidenceThreshold
    );

    // Manipulation threshold validation - must be positive and within detection limits
    // Zero threshold would disable manipulation detection, excessive thresholds could miss attacks
    require!(
        config.manipulation_threshold > 0
            && config.manipulation_threshold <= MAX_MANIPULATION_THRESHOLD,
        StateError::InvalidManipulationThreshold
    );

    // Emergency admin validation - must not be default key to ensure fail-safe capability
    require!(
        config.emergency_admin != Pubkey::default(),
        StateError::InvalidEmergencyAdmin
    );

    // Phase 3: Governance Configuration Validation
    // Ensures governance system is properly configured for decentralized control
    let governance_config = &config.governance_config;

    // Member count validation - must have at least one member but not exceed system limits
    require!(
        governance_config.member_count > 0
            && governance_config.member_count <= MAX_MULTISIG_MEMBERS as u8,
        StateError::InvalidMemberCount
    );

    // Multisig threshold validation - must be achievable but provide meaningful security
    require!(
        governance_config.multisig_threshold > 0
            && governance_config.multisig_threshold <= governance_config.member_count,
        StateError::InvalidMultisigThreshold
    );

    // Timing parameter validation - ensures governance operations have reasonable timeframes
    require!(
        governance_config.voting_period > 0 && governance_config.execution_delay >= 0,
        StateError::InvalidTimingParameters
    );

    // Quorum validation - ensures meaningful participation requirements for valid votes
    require!(
        governance_config.quorum_threshold > 0
            && governance_config.quorum_threshold <= MAX_QUORUM_THRESHOLD,
        StateError::InvalidQuorumThreshold
    );

    // Proposal threshold validation - economic barrier to prevent spam proposals
    require!(
        governance_config.proposal_threshold > 0,
        StateError::InvalidProposalThreshold
    );

    // Member and authority validation - ensures governance is properly bootstrapped
    validate_initial_members_and_authority_admin(
        &governance_config.initial_members,
        governance_config.member_count,
        &ctx.accounts.authority.key(),
        &governance_config.member_permissions,
    )?;

    // Phase 4: Account Initialization and State Setup
    // Initialize all accounts with validated configuration parameters

    let mut oracle_state = ctx.accounts.oracle_state.load_init()?;
    let mut governance_state = ctx.accounts.governance_state.load_init()?;
    let mut historical_chunk_0 = ctx.accounts.historical_chunk_0.load_init()?;
    let mut historical_chunk_1 = ctx.accounts.historical_chunk_1.load_init()?;
    let mut historical_chunk_2 = ctx.accounts.historical_chunk_2.load_init()?;

    // Oracle state initialization with comprehensive configuration
    oracle_state.authority = ctx.accounts.authority.key();
    oracle_state.version = Version {
        major: 0,
        minor: 1,
        patch: 0,
        _padding: 0,
    };

    // Initialize state flags and configure circuit breaker if enabled
    oracle_state.flags = StateFlags::new();
    if config.enable_circuit_breaker {
        oracle_state.flags.set(StateFlags::CIRCUIT_BREAKER_ENABLED);
    }

    // Initialize price data with default values - will be populated by first price update
    oracle_state.current_price = PriceData::default();
    oracle_state.twap_window = config.twap_window;
    oracle_state.current_chunk_index = 0; // Start with first historical chunk
    oracle_state.max_chunk_size = BUFFER_SIZE as u16;
    oracle_state.confidence_threshold = config.confidence_threshold;
    oracle_state.manipulation_threshold = config.manipulation_threshold;
    oracle_state.asset_seed = config.asset_seed;

    // Store PDA bumps for future address validation
    oracle_state.bump = ctx.bumps.oracle_state;
    oracle_state.governance_bump = ctx.bumps.governance_state;

    // Establish links to historical chunks for circular buffer management
    oracle_state.historical_chunks[0] = ctx.accounts.historical_chunk_0.key();
    oracle_state.historical_chunks[1] = ctx.accounts.historical_chunk_1.key();
    oracle_state.historical_chunks[2] = ctx.accounts.historical_chunk_2.key();

    oracle_state.emergency_admin = config.emergency_admin;
    oracle_state.last_update = 0; // No updates yet

    // Governance state initialization with comprehensive parameters
    governance_state.proposal_threshold = governance_config.proposal_threshold;
    governance_state.voting_period = governance_config.voting_period;
    governance_state.execution_delay = governance_config.execution_delay;
    governance_state.timelock_duration = governance_config.execution_delay; // Initial timelock matches execution delay
    governance_state.veto_period = DEFAULT_VETO_PERIOD;
    governance_state.quorum_threshold = governance_config.quorum_threshold;
    governance_state.multi_sig_threshold = governance_config.multisig_threshold;
    governance_state.active_member_count = governance_config.member_count;
    governance_state.bump = ctx.bumps.governance_state;
    governance_state.oracle_state = ctx.accounts.oracle_state.key();

    // Initialize governance members and permissions
    // Using fixed-size arrays to enable zero-copy access patterns
    for i in 0..MAX_MULTISIG_MEMBERS {
        if i < governance_config.member_count as usize {
            governance_state.multisig_members[i] = governance_config.initial_members[i];
            governance_state.member_permissions[i] = governance_config.member_permissions[i];
        } else {
            // Clear unused slots with default values for security
            governance_state.multisig_members[i] = Pubkey::default();
            governance_state.member_permissions[i] = Permissions::default();
        }
    }

    // Historical chunk initialization - establish circular buffer structure
    // Each chunk is initialized with default price points and linked to the next chunk

    historical_chunk_0.chunk_id = 0;
    historical_chunk_0.creation_timestamp = timestamp_now;
    historical_chunk_0.price_points = [PricePoint::default(); BUFFER_SIZE];
    historical_chunk_0.next_chunk = ctx.accounts.historical_chunk_1.key(); // Points to chunk 1
    historical_chunk_0.oracle_state = ctx.accounts.oracle_state.key();
    historical_chunk_0.bump = ctx.bumps.historical_chunk_0;

    historical_chunk_1.chunk_id = 1;
    historical_chunk_1.creation_timestamp = timestamp_now;
    historical_chunk_1.price_points = [PricePoint::default(); BUFFER_SIZE];
    historical_chunk_1.next_chunk = ctx.accounts.historical_chunk_2.key(); // Points to chunk 2
    historical_chunk_1.oracle_state = ctx.accounts.oracle_state.key();
    historical_chunk_1.bump = ctx.bumps.historical_chunk_1;

    historical_chunk_2.chunk_id = 2;
    historical_chunk_2.creation_timestamp = timestamp_now;
    historical_chunk_2.price_points = [PricePoint::default(); BUFFER_SIZE];
    historical_chunk_2.next_chunk = Pubkey::default(); // End of circular buffer for now
    historical_chunk_2.oracle_state = ctx.accounts.oracle_state.key();
    historical_chunk_2.bump = ctx.bumps.historical_chunk_2;

    // Phase 5: Event Emission for Transparency and Monitoring
    // Emit comprehensive initialization event for off-chain monitoring and indexing
    emit!(OracleInitialized {
        oracle_state: ctx.accounts.oracle_state.key(),
        asset_id: canonical_asset_id,
        authority: ctx.accounts.authority.key(),
        emergency_admin: config.emergency_admin,
        twap_window: config.twap_window,
        confidence_threshold: config.confidence_threshold,
        manipulation_threshold: config.manipulation_threshold,
        governance_members: governance_config.member_count,
        multisig_threshold: governance_config.multisig_threshold,
    });

    Ok(())
}
