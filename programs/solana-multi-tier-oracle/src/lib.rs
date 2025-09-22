#![allow(unexpected_cfgs)]
use anchor_lang::prelude::*;

pub mod error;
pub mod utils;
pub mod state;
pub mod components;
pub mod instructions;

use instructions::*;


declare_id!("4CVNsAY1CA9nANqBGJ4BBJAcUvPR2eTbidLu3nMewPad");

#[program]
pub mod solana_multi_tier_oracle {
    use super::*;

    pub fn initialize_oracle(ctx: Context<InitializeOracle>, config: OracleConfig) -> Result<()> {
        instructions::initialize_oracle::initialize_oracle(ctx, config)
    }

    pub fn register_price_feed(ctx: Context<RegisterPriceFeed>, feed_config: PriceFeedConfig) -> Result<()> {
        instructions::register_price_feed::register_price_feed(ctx, feed_config)
    }

    pub fn update_price(ctx: Context<UpdatePrice>, config: UpdatePriceConfig) -> Result<()> {
        instructions::update_price::update_price(ctx, config)
    }
}