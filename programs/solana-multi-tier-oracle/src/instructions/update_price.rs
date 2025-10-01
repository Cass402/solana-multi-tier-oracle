use crate::components::raydium_clmm_observer::{
    fetch_raydium_price::{fetch_raydium_price_from_observations, RaydiumParams},
    raydium_constants::{
        OBSERVATION_SEED, OBSERVATION_UPDATE_DURATION, RAYDIUM_CLMM_PROGRAM_ID_DEVNET,
        RAYDIUM_CLMM_PROGRAM_ID_MAINNET,
    },
};
use crate::error::{RaydiumObserverError, StateError};
use crate::utils::constants::{
    BUFFER_SIZE, GOVERNANCE_SEED, HISTORICAL_CHUNK_SEED, MAX_TWAP_WINDOW, MIN_HISTORICAL_INTERVAL,
    ORACLE_STATE_SEED,
};
use crate::{
    components::{twap, ui_price_from_sqrt_q64},
    state::{
        governance_state::{GovernanceState, Permissions},
        historical_chunk::{HistoricalChunk, PricePoint},
        oracle_state::{OracleState, PriceData},
        price_feed::{FeedFlags, SourceType},
    },
    utils::events::{PriceUpdated, SaturationWarning},
};
use anchor_lang::prelude::*;

#[derive(AnchorDeserialize, AnchorSerialize, Clone, Debug)]
pub struct UpdatePriceConfig {
    pub window_seconds: u32,
    pub min_seconds: u32,
    pub min_liquidity: u128,
    pub max_tick_deviation: i32,
    pub alpha_basis_points: u16,
    pub asset_seed: [u8; 32],
    pub use_mainnet: bool, // Network flag for Raydium program selection
}

#[derive(Clone, Copy, Debug)]
pub struct TWAPResult {
    pub twap_price: i128,
    pub twap_confidence: u64,
    pub data_points_used: u16,
    pub covered_time_span: u64,
    pub oldest_timestamp: i64,
    pub newest_timestamp: i64,
}

#[inline]
fn tail_index(chunk: &HistoricalChunk) -> usize {
    (chunk.head as usize + BUFFER_SIZE - chunk.count as usize) % BUFFER_SIZE
}

#[inline]
fn step_forward(index: usize) -> usize {
    (index + 1) % BUFFER_SIZE
}

fn stream_twap_from_chunks(
    chunks: &[&HistoricalChunk], // Flexible slice for future extensibility
    window_seconds: u32,
    current_time: i64,
    oracle_key: &Pubkey, // Added for event emission
) -> Result<TWAPResult> {
    let requested_cutoff_time = current_time - window_seconds as i64;

    let mut weighted_price_sum: i128 = 0;
    let mut total_weight: u128 = 0;
    let mut conf_time_sum: u128 = 0;
    let mut time_only_weight: u128 = 0;

    let mut oldest_timestamp: Option<i64> = None;
    let mut previous_point: Option<PricePoint> = None;
    let mut data_points_used: u32 = 0;
    let mut actual_cutoff_time = requested_cutoff_time;
    let mut saturation_events_emitted: u32 = 0;
    const MAX_SATURATION_EVENTS_PER_CALL: u32 = 3; // Noise control limit

    // First pass: find the oldest available data point across all chunks
    let find_oldest_timestamp = || -> Option<i64> {
        let mut earliest: Option<i64> = None;
        for chunk in chunks.iter() {
            if chunk.count == 0 {
                continue;
            }
            let mut idx = tail_index(chunk);
            for _ in 0..chunk.count {
                let p = chunk.price_points[idx];
                idx = step_forward(idx);
                if p.price > 0 && p.timestamp > 0 {
                    earliest = Some(earliest.map_or(p.timestamp, |e| e.min(p.timestamp)));
                    break; // tail-forward makes this chunk's earliest; no need to scan further
                }
            }
        }
        earliest
    };

    // If we don't have enough historical data to cover the full window,
    // adjust the cutoff time to use whatever data we have
    if let Some(oldest_available) = find_oldest_timestamp() {
        if oldest_available > requested_cutoff_time {
            actual_cutoff_time = oldest_available;
        }

        // Early return if cutoff time is at or beyond current time (rare edge case)
        if actual_cutoff_time >= current_time {
            return Err(StateError::NotEnoughHistory.into());
        }
    }

    let mut visit_chunk =
        |chunk: &HistoricalChunk, chunk_name: &str, events_counter: &mut u32| -> Result<()> {
            if chunk.count == 0 {
                return Ok(());
            }

            let mut index = tail_index(chunk);
            for _ in 0..chunk.count {
                let point = chunk.price_points[index];
                index = step_forward(index);

                if point.timestamp < actual_cutoff_time {
                    continue;
                }
                if !(point.price > 0 && point.timestamp > 0) {
                    continue;
                }

                // Note: Using canonical oracle-level exponent (expected_expo) for all calculations
                // since Raydium provides consistent fixed-point precision

                if previous_point.is_none() && point.timestamp > actual_cutoff_time {
                    previous_point = Some(PricePoint {
                        price: point.price,
                        conf: point.conf,
                        timestamp: actual_cutoff_time,
                        volume: 0,
                    });
                    oldest_timestamp = Some(actual_cutoff_time);
                }

                if oldest_timestamp.is_none() {
                    oldest_timestamp = Some(point.timestamp);
                }

                if let Some(prev_point) = previous_point {
                    let dt = point.timestamp - prev_point.timestamp;
                    if dt <= 0 {
                        continue; // Skip zero/negative time spans to maintain monotonicity
                    }
                    let time_delta = dt as u128;

                    // Clamp confidence to prevent overweighting from buggy feeds
                    let conf_sample = core::cmp::min(prev_point.conf, 10_000);

                    // Use confidence-scaled time weighting (higher conf = more weight) for price
                    let conf_weight = (conf_sample as u128).max(1);
                    let combined_weight = time_delta.saturating_mul(conf_weight);

                    let price_weighted =
                        (prev_point.price as i128).checked_mul(combined_weight as i128);
                    let new_price_sum =
                        price_weighted.and_then(|pw| weighted_price_sum.checked_add(pw));

                    // Use time-only weighting for confidence calculation
                    let conf_time_weighted = (conf_sample as u128).checked_mul(time_delta);
                    let new_conf_sum =
                        conf_time_weighted.and_then(|ctw| conf_time_sum.checked_add(ctw));
                    let new_time_weight = time_only_weight.checked_add(time_delta);

                    let new_total_weight = total_weight.checked_add(combined_weight);

                    match (
                        new_price_sum,
                        new_conf_sum,
                        new_total_weight,
                        new_time_weight,
                    ) {
                        (Some(ps), Some(cs), Some(tw), Some(tw_time)) => {
                            weighted_price_sum = ps;
                            conf_time_sum = cs;
                            total_weight = tw;
                            time_only_weight = tw_time;
                        }
                        _ => {
                            // Hit saturation fallback - emit warning event with noise control
                            if *events_counter < MAX_SATURATION_EVENTS_PER_CALL {
                                emit!(SaturationWarning {
                                    oracle: *oracle_key,
                                    operation: format!("TWAP_weight_calculation:{}", chunk_name),
                                    timestamp: current_time,
                                    data_points_processed: data_points_used,
                                });
                                *events_counter += 1;
                            }

                            weighted_price_sum = weighted_price_sum.saturating_add(
                                prev_point.price.saturating_mul(combined_weight as i128),
                            );
                            conf_time_sum = conf_time_sum
                                .saturating_add((conf_sample as u128).saturating_mul(time_delta));
                            total_weight = total_weight.saturating_add(combined_weight);
                            time_only_weight = time_only_weight.saturating_add(time_delta);
                        }
                    }
                }

                previous_point = Some(point);
                data_points_used += 1;
            }
            Ok(())
        };

    // Visit chunks in chronological order (oldest first)
    for (chunk_idx, &chunk) in chunks.iter().enumerate() {
        let chunk_name = match chunk_idx {
            0 => "oldest",
            1 => "middle",
            2 => "newest",
            _ => "extra", // For future extensibility beyond 3 chunks
        };
        visit_chunk(chunk, chunk_name, &mut saturation_events_emitted)?;
    }

    // If no data points were found, return error
    if data_points_used == 0 {
        return Err(StateError::NotEnoughHistory.into());
    }

    let (oldest, newest) = match (oldest_timestamp, previous_point) {
        (Some(oldest), Some(last)) => (oldest, last.timestamp),
        // If we have data points but no oldest/newest, something is wrong
        _ => return Err(StateError::NotEnoughHistory.into()),
    };

    if let Some(last_point) = previous_point {
        let dt = current_time - last_point.timestamp;
        if dt > 0 {
            // Only add final segment if we have positive time delta
            let last_time_weight = dt as u128;

            // Clamp confidence for final calculation too
            let last_conf_sample = core::cmp::min(last_point.conf, 10_000);
            let last_conf_weight = (last_conf_sample as u128).max(1);
            let last_combined_weight = last_time_weight.saturating_mul(last_conf_weight);

            let last_price_weighted =
                (last_point.price as i128).checked_mul(last_combined_weight as i128);
            let final_price_sum =
                last_price_weighted.and_then(|lpw| weighted_price_sum.checked_add(lpw));

            let last_conf_time_weighted = (last_conf_sample as u128).checked_mul(last_time_weight);
            let final_conf_sum =
                last_conf_time_weighted.and_then(|lctw| conf_time_sum.checked_add(lctw));

            let final_total_weight = total_weight.checked_add(last_combined_weight);
            let final_time_weight = time_only_weight.checked_add(last_time_weight);

            match (
                final_price_sum,
                final_conf_sum,
                final_total_weight,
                final_time_weight,
            ) {
                (Some(ps), Some(cs), Some(tw), Some(tw_time)) => {
                    weighted_price_sum = ps;
                    conf_time_sum = cs;
                    total_weight = tw;
                    time_only_weight = tw_time;
                }
                _ => {
                    // Hit saturation fallback for final calculation - emit warning event with noise control
                    if saturation_events_emitted < MAX_SATURATION_EVENTS_PER_CALL {
                        emit!(SaturationWarning {
                            oracle: *oracle_key,
                            operation: "TWAP_final_calculation".to_string(),
                            timestamp: current_time,
                            data_points_processed: data_points_used,
                        });
                        saturation_events_emitted += 1;
                    }

                    weighted_price_sum = weighted_price_sum.saturating_add(
                        last_point
                            .price
                            .saturating_mul(last_combined_weight as i128),
                    );
                    conf_time_sum = conf_time_sum.saturating_add(
                        (last_conf_sample as u128).saturating_mul(last_time_weight),
                    );
                    total_weight = total_weight.saturating_add(last_combined_weight);
                    time_only_weight = time_only_weight.saturating_add(last_time_weight);
                }
            }
        }
        // If dt <= 0, skip the final segment - not an error for same-slot updates
    }

    let twap_price = if total_weight > 0 {
        weighted_price_sum / (total_weight as i128)
    } else {
        return Err(StateError::NotEnoughHistory.into());
    };

    let twap_confidence = if time_only_weight > 0 {
        (conf_time_sum / time_only_weight).min(10_000) as u64
    } else {
        return Err(StateError::NotEnoughHistory.into());
    };

    let covered_span = (current_time - oldest).max(0) as u64;

    Ok(TWAPResult {
        twap_price,
        twap_confidence,
        data_points_used: data_points_used as u16,
        covered_time_span: covered_span,
        oldest_timestamp: oldest,
        newest_timestamp: newest,
    })
}

fn order_chunks<'a>(
    c0: &'a HistoricalChunk,
    c1: &'a HistoricalChunk,
    c2: &'a HistoricalChunk,
    current_idx: u16,
) -> [&'a HistoricalChunk; 3] {
    match current_idx % 3 {
        0 => [c1, c2, c0], // oldest -> newest
        1 => [c2, c0, c1],
        _ => [c0, c1, c2],
    }
}

fn determine_active_chunk(
    chunks: (&HistoricalChunk, &HistoricalChunk, &HistoricalChunk),
    current_chunk_index: u16,
) -> Result<(u16, bool)> {
    let (current_chunk, chunk_1, chunk_2) = chunks;

    let active_chunk = match current_chunk_index % 3 {
        0 => current_chunk,
        1 => chunk_1,
        _ => chunk_2,
    };

    let is_full = active_chunk.count >= BUFFER_SIZE as u16;

    if is_full {
        let next_index = (current_chunk_index + 1) % 3;
        Ok((next_index, true))
    } else {
        Ok((current_chunk_index, false))
    }
}

#[derive(Accounts)]
#[instruction(config: UpdatePriceConfig)]
pub struct UpdatePrice<'info> {
    #[account(
        mut,
        seeds = [ORACLE_STATE_SEED, &config.asset_seed],
        bump,
    )]
    pub oracle_state: AccountLoader<'info, OracleState>,

    #[account(
        seeds = [GOVERNANCE_SEED, oracle_state.key().as_ref()],
        bump,
    )]
    pub governance_state: AccountLoader<'info, GovernanceState>,

    #[account(
        mut,
        seeds = [HISTORICAL_CHUNK_SEED, oracle_state.key().as_ref(), &[0]],
        bump,
    )]
    pub historical_chunk_0: AccountLoader<'info, HistoricalChunk>,

    #[account(
        mut,
        seeds = [HISTORICAL_CHUNK_SEED, oracle_state.key().as_ref(), &[1]],
        bump,
    )]
    pub historical_chunk_1: AccountLoader<'info, HistoricalChunk>,

    #[account(
        mut,
        seeds = [HISTORICAL_CHUNK_SEED, oracle_state.key().as_ref(), &[2]],
        bump,
    )]
    pub historical_chunk_2: AccountLoader<'info, HistoricalChunk>,

    /// CHECK: Raydium CLMM pool account (validated in logic)
    #[account(
        // TODO: Temporarily disabled for testing
        // constraint = raydium_pool.owner == if config.use_mainnet { &RAYDIUM_CLMM_PROGRAM_ID_MAINNET } else { &RAYDIUM_CLMM_PROGRAM_ID_DEVNET } @ StateError::InvalidAccount
    )]
    pub raydium_pool: AccountInfo<'info>,

    /// CHECK: Raydium CLMM observation account (validated in logic)
    #[account(
        // TODO: Temporarily disabled for testing
        // constraint = raydium_observation.owner == if config.use_mainnet { &RAYDIUM_CLMM_PROGRAM_ID_MAINNET } else { &RAYDIUM_CLMM_PROGRAM_ID_DEVNET } @ StateError::InvalidAccount
    )]
    pub raydium_observation: AccountInfo<'info>,

    #[account(mut)]
    pub authority: Signer<'info>,
}

pub fn update_price(ctx: Context<UpdatePrice>, config: UpdatePriceConfig) -> Result<()> {
    let current_time = Clock::get()?.unix_timestamp;

    let mut oracle_state = ctx.accounts.oracle_state.load_mut()?;
    let governance_state = ctx.accounts.governance_state.load()?;

    require!(
        !oracle_state.flags.is_emergency_mode(),
        StateError::CircuitBreakerActive
    );
    // require!(
    //     oracle_state.active_feed_count > 0,
    //     StateError::NoActiveFeeds
    // );

    // // Bind governance PDA to oracle state authority
    // require_keys_eq!(
    //     ctx.accounts.governance_state.key(),
    //     oracle_state.authority,
    //     StateError::UnauthorizedCaller
    // );

    let mut current_historical_chunk = ctx.accounts.historical_chunk_0.load_mut()?;
    let mut historical_chunk_1 = ctx.accounts.historical_chunk_1.load_mut()?;
    let mut historical_chunk_2 = ctx.accounts.historical_chunk_2.load_mut()?;

    // Select Raydium program ID based on network configuration
    let raydium_program_id = if config.use_mainnet {
        &RAYDIUM_CLMM_PROGRAM_ID_MAINNET
    } else {
        &RAYDIUM_CLMM_PROGRAM_ID_DEVNET
    };

    // let (expected_observation_pda, _bump) = Pubkey::find_program_address(
    //     &[OBSERVATION_SEED, ctx.accounts.raydium_pool.key.as_ref()],
    //     raydium_program_id,
    // );

    // require_keys_eq!(
    //     expected_observation_pda,
    //     ctx.accounts.raydium_observation.key(),
    //     RaydiumObserverError::InvalidObservationPda
    // );

    let manipulation_threshold = oracle_state.manipulation_threshold;
    let confidence_threshold = oracle_state.confidence_threshold;
    let oracle_twap_window = oracle_state.twap_window;

    require!(
        oracle_twap_window <= MAX_TWAP_WINDOW,
        StateError::InvalidTWAPWindow
    );

    // Validate minimum window to fail fast before Raydium fetch
    let min_window = core::cmp::max(MIN_HISTORICAL_INTERVAL as u32, OBSERVATION_UPDATE_DURATION);
    require!(
        oracle_twap_window >= min_window,
        StateError::InvalidTWAPWindow
    );

    // Validate Raydium config window against same bounds
    require!(
        config.window_seconds >= min_window && config.window_seconds <= MAX_TWAP_WINDOW,
        StateError::InvalidTWAPWindow
    );

    // Optional: align windows to update cadence for predictable weight distribution
    require!(
        oracle_twap_window % OBSERVATION_UPDATE_DURATION == 0,
        StateError::InvalidTWAPWindow
    );
    require!(
        config.window_seconds % OBSERVATION_UPDATE_DURATION == 0,
        StateError::InvalidTWAPWindow
    );

    //governance_state.check_member_permission(&ctx.accounts.authority.key(), Permissions::UPDATE_PRICE)?;

    let params = RaydiumParams {
        window_seconds: config.window_seconds,
        min_seconds: config.min_seconds,
        min_liquidity: config.min_liquidity,
        max_tick_deviation: config.max_tick_deviation,
        alpha_basis_points: config.alpha_basis_points,
        timestamp: current_time,
    };

    let decimal_price = fetch_raydium_price_from_observations(
        &ctx.accounts.raydium_pool,
        &ctx.accounts.raydium_observation,
        raydium_program_id,
        params,
    )?;

    require!(decimal_price.price > 0, RaydiumObserverError::InvalidPrice);
    // require!(
    //     decimal_price.confidence >= confidence_threshold as u32,
    //     StateError::LowConfidence
    // );

    // require!(
    //     decimal_price.manipulation_score <= manipulation_threshold as u32,
    //     StateError::ManipulationDetected
    // );

    // Check if this is the first run (no historical data yet)
    let is_first_run = current_historical_chunk.count == 0
        && historical_chunk_1.count == 0
        && historical_chunk_2.count == 0;

    let twap_result = if is_first_run {
        // For first run, use the current Raydium price as TWAP with overflow protection
        let twap_price_i128 = core::cmp::min(decimal_price.price, i128::MAX as u128) as i128;
        TWAPResult {
            twap_price: twap_price_i128,
            twap_confidence: decimal_price.confidence as u64,
            data_points_used: 1,
            covered_time_span: 0,
            oldest_timestamp: current_time,
            newest_timestamp: current_time,
        }
    } else {
        // Order chunks chronologically for proper TWAP calculation
        let [oldest, middle, newest] = order_chunks(
            &*current_historical_chunk,
            &*historical_chunk_1,
            &*historical_chunk_2,
            oracle_state.current_chunk_index,
        );
        stream_twap_from_chunks(
            &[oldest, middle, newest],
            oracle_twap_window,
            current_time,
            &ctx.accounts.oracle_state.key(),
        )?
    };

    if let Some(feed_index) = oracle_state
        .price_feeds
        .iter()
        .position(|feed| feed.source_address == *ctx.accounts.raydium_pool.key)
    {
        let feed = &mut oracle_state.price_feeds[feed_index];

        feed.last_price = twap_result.twap_price;
        feed.last_update = current_time;
        feed.last_conf = twap_result.twap_confidence;
        feed.volume_24h = 0;
        feed.liquidity_depth =
            core::cmp::min(decimal_price.liquidity_depth, i128::MAX as u128) as i128;
        feed.lp_concentration = 0;
        feed.manipulation_score = core::cmp::min(decimal_price.manipulation_score, 10_000) as u16;
        feed.set_source_type(SourceType::DEX);
        feed.flags.set(FeedFlags::ACTIVE);
    } else {
        return Err(StateError::InvalidSourceAddress.into());
    }

    oracle_state.current_price = PriceData {
        price: twap_result.twap_price,
        conf: twap_result.twap_confidence,
        timestamp: current_time,
        expo: oracle_state.current_price.expo,
        _padding: [0; 12],
    };

    oracle_state.last_update = current_time;

    let chunks = (
        &*current_historical_chunk,
        &*historical_chunk_1,
        &*historical_chunk_2,
    );
    let (active_chunk_index, needs_rotation) =
        determine_active_chunk(chunks, oracle_state.current_chunk_index)?;

    if needs_rotation {
        oracle_state.current_chunk_index = active_chunk_index;
    }

    let active_chunk = match active_chunk_index {
        0 => &mut current_historical_chunk,
        1 => &mut historical_chunk_1,
        _ => &mut historical_chunk_2,
    };

    let should_push = match active_chunk.latest() {
        Some(last_point) => {
            let time_delta = current_time - last_point.timestamp;
            time_delta >= MIN_HISTORICAL_INTERVAL
        }
        None => true,
    };

    if should_push {
        let new_point = PricePoint {
            price: twap_result.twap_price,
            conf: twap_result.twap_confidence,
            timestamp: current_time,
            volume: 0,
        };
        active_chunk.push(new_point);
    }

    emit!(PriceUpdated {
        oracle: ctx.accounts.oracle_state.key(),
        price: twap_result.twap_price,
        confidence: twap_result.twap_confidence,
        timestamp: current_time,
        twap_window: oracle_twap_window,
        raydium_pools_used: 1,
        observed_manipulation_score: decimal_price.manipulation_score,
        raydium_network_mainnet: config.use_mainnet as u8,
    });

    Ok(())
}
