use anchor_lang::prelude::*;
use crate::error::StateError;
use crate::state::oracle_state::OracleState;
use crate::state::price_feed::{PriceFeed, FeedFlags, SourceType};
use crate::state::governance_state::{GovernanceState, Permissions};
use crate::utils::constants::{ORACLE_STATE_SEED, GOVERNANCE_SEED, MAX_FEED_WEIGHT, MIN_CLMM_LIQUIDITY, MIN_AMM_LIQUIDITY, MAX_PRICE_FEEDS, WEIGHT_PRECISION};
use crate::utils::events::PriceFeedRegistered;

#[derive(AnchorSerialize, AnchorDeserialize, Clone)]
pub struct PriceFeedConfig {
    pub source_address: Pubkey,
    pub source_type: SourceType,
    pub weight: u16,
    pub min_liquidity: u128,
    pub staleness_threshold: u32,
    pub asset_seed: [u8; 32],
}

#[derive(Clone, Copy)]
struct ValidationResult {
    pub is_valid: bool,
    pub error_flags: u8,
}

impl ValidationResult {
    const ERROR_DUPLICATE_SOURCE: u8 = 1 << 0;
    const ERROR_EXCESSIVE_WEIGHT: u8 = 1 << 1;
    const ERROR_UNAUTHORIZED_PROGRAM: u8 = 1 << 2;
    const ERROR_INVALID_WEIGHT: u8 = 1 << 3;
    const ERROR_INSUFFICIENT_LIQUIDITY: u8 = 1 << 4;
    //const ERROR_STALENESS_OUT_OF_RANGE: u8 = 1 << 5;

    fn success() -> Self {
        Self { is_valid: true, error_flags: 0 }
    }

    fn with_error(error_flag: u8) -> Self {
        Self { is_valid: false, error_flags: error_flag }
    }
    
    fn add_error(&mut self, error_flag: u8) {
        self.is_valid = false;
        self.error_flags |= error_flag;
    }
}

impl PriceFeedConfig {
    fn validate_weight(&self) -> ValidationResult {
        if self.weight == 0 || self.weight > MAX_FEED_WEIGHT {
            ValidationResult::with_error(ValidationResult::ERROR_INVALID_WEIGHT)
        } else {
            ValidationResult::success()
        }
    }

    fn validate_source_address(&self) -> ValidationResult {
        match self.source_type {
            SourceType::DEX => {
                if self.min_liquidity < MIN_CLMM_LIQUIDITY as u128{
                    ValidationResult::with_error(ValidationResult::ERROR_INSUFFICIENT_LIQUIDITY)
                } else {
                    ValidationResult::success()
                }
            },

            SourceType::CEX => {
                ValidationResult::success()
            },

            SourceType::Oracle => {
                ValidationResult::success()
            },

            SourceType::Aggregator => {
                if self.min_liquidity < MIN_AMM_LIQUIDITY as u128 {
                    ValidationResult::with_error(ValidationResult::ERROR_INSUFFICIENT_LIQUIDITY)
                } else {
                    ValidationResult::success()
                }
            },
        }
    }
}

struct ValidationContext<'a> {
    oracle_state: &'a OracleState,
    current_total_weight: u32,
    active_feed_count: u8,
}

impl<'a> ValidationContext<'a> {
    fn new(oracle_state: &'a OracleState) -> Result<Self> {
        let current_total_weight = oracle_state.active_feeds()
            .iter()
            .try_fold(0u32, |acc, feed| {
                acc.checked_add(feed.weight as u32)
                .ok_or(StateError::ExcessiveTotalWeight)
            })?;

        Ok(Self {
            oracle_state,
            current_total_weight,
            active_feed_count: oracle_state.active_feed_count,
        })
    }

    fn validate_oracle_constraints(&self) -> Result<()> {
        if self.active_feed_count >= MAX_PRICE_FEEDS as u8 {
            return Err(StateError::TooManyFeeds.into());
        }

        if self.oracle_state.is_circuit_breaker_enabled() {
            return Err(StateError::CircuitBreakerActive.into());
        }

        Ok(())
    }

    fn has_duplicate_source(&self, source_address: &Pubkey) -> bool {
        self.oracle_state.active_feeds()
            .iter()
            .any(|feed| &feed.source_address == source_address)
    }

    fn validate_total_weight(&self, new_weight: u16) -> Result<ValidationResult> {
        let new_total_weight = self.current_total_weight
            .checked_add(new_weight as u32)
            .ok_or(StateError::ExcessiveTotalWeight)?;

        if new_total_weight > WEIGHT_PRECISION {
            Ok(ValidationResult::with_error(ValidationResult::ERROR_EXCESSIVE_WEIGHT))
        } else {
            Ok(ValidationResult::success())
        }
    }
}

fn convert_validation_error(error_flags: u8) -> StateError {
    if error_flags & ValidationResult::ERROR_DUPLICATE_SOURCE != 0 {
        StateError::DuplicateFeedSource
    } else if error_flags & ValidationResult::ERROR_EXCESSIVE_WEIGHT != 0 {
        StateError::ExcessiveTotalWeight
    } else if error_flags & ValidationResult::ERROR_UNAUTHORIZED_PROGRAM != 0 {
        StateError::UnauthorizedFeedRegistration
    } else if error_flags & ValidationResult::ERROR_INVALID_WEIGHT != 0 {
        StateError::InvalidFeedWeight
    } else if error_flags & ValidationResult::ERROR_INSUFFICIENT_LIQUIDITY != 0 {
        StateError::InsufficientSourceLiquidity
    //} else if error_flags & ValidationResult::ERROR_STALENESS_OUT_OF_RANGE != 0 {
    //    StateError::ExcessiveExternalStaleness
    } else {
        StateError::InvalidSourceAddress // Fallback error
    }
}

fn validate_source_program_ownership(
    feed_source: &UncheckedAccount,
    source_type: SourceType,
    governance_state: &GovernanceState,
) -> ValidationResult {
    match source_type {
        SourceType::DEX | SourceType::CEX => {
            if governance_state.strict_mode_enabled == 1 {
                let owner = *feed_source.owner;
                let is_allowed = governance_state.allowed_dex_programs
                    .iter()
                    .take(governance_state.allowed_dex_program_count as usize)
                    .any(|&program| program == owner);

                if !is_allowed {
                    msg!("Unauthorized DEX/CEX program: {}", owner);
                    return ValidationResult::with_error(ValidationResult::ERROR_UNAUTHORIZED_PROGRAM);
                }
            }
            ValidationResult::success()
        },

        SourceType::Aggregator => {
            if governance_state.strict_mode_enabled == 1 {
                let owner = *feed_source.owner;
                let is_allowed = governance_state.allowed_aggregator_programs
                    .iter()
                    .take(governance_state.allowed_aggregator_program_count as usize)
                    .any(|&program| program == owner);

                if !is_allowed {
                    msg!("Unauthorized Aggregator program: {}", owner);
                    return ValidationResult::with_error(ValidationResult::ERROR_UNAUTHORIZED_PROGRAM);
                }
            }
            ValidationResult::success()
        },

        SourceType::Oracle => {
            // Oracles are not implemented yet, so skip validation for now
            ValidationResult::success()
        },
    }
}

fn validate_feed_registration(
    ctx: &ValidationContext,
    feed_config: &PriceFeedConfig,
    feed_source: &UncheckedAccount,
    governance_state: &GovernanceState,
) -> Result<()> {
    if ctx.has_duplicate_source(&feed_config.source_address) {
        return Err(StateError::DuplicateFeedSource.into());
    }

    let weight_result = feed_config.validate_weight();
    if !weight_result.is_valid {
        return Err(convert_validation_error(weight_result.error_flags).into());
    }

    let total_weight_result = ctx.validate_total_weight(feed_config.weight)?;
    if !total_weight_result.is_valid {
        return Err(convert_validation_error(total_weight_result.error_flags).into());
    }

    let source_result = feed_config.validate_source_address();
    if !source_result.is_valid {
        return Err(convert_validation_error(source_result.error_flags).into());
    }

    let program_result = validate_source_program_ownership(feed_source, feed_config.source_type, governance_state);
    if !program_result.is_valid {
        return Err(convert_validation_error(program_result.error_flags).into());
    }

    Ok(())
}

fn create_price_feed(feed_config: &PriceFeedConfig, timestamp: i64) -> PriceFeed {
    let mut flags = FeedFlags::new();
    flags.set(FeedFlags::ACTIVE);

    PriceFeed {
        source_address: feed_config.source_address,
        last_price: 0,
        volume_24h: 0,
        liquidity_depth: 0,
        last_conf: 0,
        last_update: timestamp,
        last_expo: 0,
        weight: feed_config.weight,
        lp_concentration: 0,
        manipulation_score: 0,
        source_type: feed_config.source_type.as_u8(),
        flags,
        _padding: [0; 4],
    }
}

#[derive(Accounts)]
#[instruction(feed_config: PriceFeedConfig)]
pub struct RegisterPriceFeed<'info> {
    #[account(
        mut,
        seeds = [ORACLE_STATE_SEED, &feed_config.asset_seed],
        bump
    )]
    pub oracle_state: AccountLoader<'info, OracleState>,

    #[account(
        seeds = [GOVERNANCE_SEED, oracle_state.key().as_ref()],
        bump
    )]
    pub governance_state: AccountLoader<'info, GovernanceState>,

    /// CHECK: This is a feed source account; validation is performed in the instruction
    #[account(
        address = feed_config.source_address @ StateError::InvalidSourceAddress
    )]
    pub feed_source: UncheckedAccount<'info>,

    #[account(mut)]
    pub authority: Signer<'info>,
}

pub fn register_price_feed(
    ctx: Context<RegisterPriceFeed>,
    feed_config: PriceFeedConfig,
) -> Result<()> {
    let timestamp_now = Clock::get()?.unix_timestamp;

    let governance_state = ctx.accounts.governance_state.load()?;
    let mut oracle_state = ctx.accounts.oracle_state.load_mut()?;

    require_keys_eq!(
        governance_state.oracle_state,
        ctx.accounts.oracle_state.key(),
        StateError::UnauthorizedCaller
    );

    let validation_context = ValidationContext::new(&oracle_state)?;

    validation_context.validate_oracle_constraints()?;

    governance_state.check_member_permission(&ctx.accounts.authority.key(), Permissions::ADD_FEED)?;

    validate_feed_registration(&validation_context, &feed_config, &ctx.accounts.feed_source, &governance_state)?;

    let final_total_weight = validation_context.current_total_weight
        .checked_add(feed_config.weight as u32)
        .ok_or(StateError::ExcessiveTotalWeight)?;

    let active_feed_count = oracle_state.active_feed_count;
    let feed_index = oracle_state.active_feed_count as usize;
    oracle_state.price_feeds[feed_index] = create_price_feed(&feed_config, timestamp_now);
    oracle_state.set_active_feed_count(active_feed_count+1)?;

    emit!(PriceFeedRegistered {
        oracle: ctx.accounts.oracle_state.key(),
        feed_address: feed_config.source_address,
        source_type: feed_config.source_type,
        weight: feed_config.weight,
        feed_index: feed_index as u32,
        total_weight: final_total_weight,
        timestamp: timestamp_now,
    });

    Ok(())
}