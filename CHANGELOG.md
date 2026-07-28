# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]
### Fixed
- **factory: `set_treasury`/`set_global_fee`/`set_global_fee_paginated` panic when any CL pool exists**
  - `sync_global_fee_page` iterated `PoolByIndex` and unconditionally called `AmmPoolClient::set_protocol_fee()` on every non-governed entry. CL pools lack `GovernanceFor` and don't implement `set_protocol_fee`, so the call would trap and revert the entire admin transaction as soon as a page containing a CL pool address was reached.
  - Added a `PoolTokens` presence guard (mirroring the existing guard in `sweep_fees_page`): entries without a `PoolTokens` key are not AMM pools and are skipped. This blocks the invalid cross-interface call both for CL pools that may have been written to `PoolByIndex` before the separate-index fix and for any future non-AMM pool types.
- **incentive_campaigns: retroactive reward gaming via flash-deposit or rate change (#425)**
  - The old `claim_rewards` formula (`reward_rate × elapsed × lp_balance / total_supply`)
    applied the provider's *current* LP balance to the *entire* elapsed window since campaign
    start, allowing a late joiner to flash-deposit and claim a disproportionate share of
    already-accrued rewards.  `set_campaign_rate` had the same flaw — changing the rate
    silently rewrote the entire reward history.
  - Replaced the naive formula with a **MasterChef-style per-second accumulator**
    (`acc_reward_per_share`, scaled by `PRECISION = 1e12`) stored on the `Campaign` struct.
    The accumulator advances by `reward_rate × Δt / total_supply` each second.  Each
    provider stores their snapshot of the accumulator at last claim; pending rewards are
    `lp_balance × (acc_now − acc_at_snapshot) / PRECISION`.
  - A flash depositor who claims in the same ledger as their deposit earns essentially
    nothing (`acc_delta ≈ 0`).  An honest LP who held since campaign start earns their
    full time-weighted share.
  - `set_campaign_rate` now flushes the accumulator to the current timestamp *before*
    updating the rate, so past accruals are locked in at the old rate and only future
    seconds use the new one.
  - `DataKey::ProviderDebt` replaced by `DataKey::ProviderSnapshot` (stores
    `ProviderSnapshot { acc_at_snapshot: i128 }` instead of a raw claimed amount).
  - Two new regression tests added: `test_flash_deposit_cannot_claim_retroactive_rewards`
    and `test_rate_change_is_not_retroactive`.
  - Existing tests updated to account for the snapshot-init step (first `claim_rewards`
    call sets the baseline and returns 0) and corrected expected amounts.

### Added
- Governance contract with multi-type parameter voting (`ProposalKind` enum covering Fee, Protocol Fee, Flash Loan Fee, Transfer Admin, Pause, and Unpause), timelocks, quorum requirements, and voting power locks (#137)
- Factory contract for deploying and registering AMM pools, featuring pool count (`get_pool_count`) and paginated pool queries (`get_pools`) (#139)
- Flash loan support with a dedicated update interface (`update_flash_loan_fee`) and configurable fees
- TWAP price accumulators via `get_price_cumulative` and a sample `TwapConsumer` contract
- Protocol fee collection (`set_protocol_fee`, `get_protocol_fee`, `withdraw_protocol_fees`)
- Emergency pause/unpause circuit breakers (`pause`, `unpause`, `is_paused`)
- Post-deployment swap fee adjustment (`update_fee`)
- Two-step administrator transfer (`propose_admin`, `accept_admin`)
- Ledger timestamp `deadline` parameter on `swap`, `swap_exact_out`, `add_liquidity`, and `remove_liquidity` for execution safety
- Detailed swap quotes (`simulate_swap`) including price impact and fee breakdown
- Reverse query quote (`get_amount_in`)
- Python client example (`examples/python/`)
- TS client example (`examples/client/`)
- Reproducible contract build environment with Docker
- Makefile with shortcuts for building, testing, linting, formatting, and end-to-end testing
- Complete machine-readable ABI schema JSON (`docs/abi.json`) (#143)
### Changed
- `reserve_manager` docs clarify the contract is off-chain-only; on-chain AMM hookup is deferred (#518).
