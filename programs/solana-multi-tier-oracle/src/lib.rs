#![allow(unexpected_cfgs)]
use anchor_lang::prelude::*;

pub mod utils;
pub mod state;

declare_id!("4CLGL8iE73T7Wcwjt3q2XapX22iSxPpXFpYGhZ33yc9h");

#[program]
pub mod solana_multi_tier_oracle {
    use super::*;

    pub fn initialize(ctx: Context<Initialize>) -> Result<()> {
        msg!("Greetings from: {:?}", ctx.program_id);
        Ok(())
    }
}

#[derive(Accounts)]
pub struct Initialize {}
