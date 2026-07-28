# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]
### Fixed
- `LpToken::unlock` previously authorised against the currently configured `DataKey::Locker`, so any `set_locker` rotation orphaned LP tokens whose locker had locked them via `LockedVote`. The unlock function now requires auth from the locker that originally locked the tokens, recorded per-locker in a new `LockEntry(Address, Address)` storage entry. Each locker retains authority over its own contribution; a freshly-set locker can only unlock tokens it itself locked. (closes #556)
- `contracts/router/Cargo.toml` and the workspace `Cargo.toml` both contained duplicate table entries (`[dependencies]`, `[dev-dependencies]`, and member list) that caused `cargo` to refuse to load the workspace entirely. Merged into single tables and removed duplicate members.

### Added
- `LpToken::migrate_legacy_lock(holder, locker, amount)` admin-only helper to migrate a holder's pre-fix `Locked(holder) > 0` balance into per-locker `LockEntry` entries after upgrading from a contract version that tracked only the total `Locked` counter.
- `LpTokenInterface::unlock` now takes an explicit `locker: Address` parameter; `governance::unlock_vote` calls it with `env.current_contract_address()` as the locker.

### Breaking
- `LpToken::unlock(holder, amount)` is replaced by `LpToken::unlock(holder, locker, amount)`. The previous locker parameter read from `DataKey::Locker` storage is now an explicit argument. External SDK clients bound to the old public ABI must switch to the new signature.

### Legacy
- `contracts/amm/src/lib.rs` references several `DataKey` enum variants that are not declared in the enum on `main` (`FeeBps`, `AccruedFeeA`, `AccruedFeeB`, `FeeRecipient`, `ProtocolFeeBps`, `FlashLoanFeeBps`, `Paused`, `Admin`, `PendingAdmin`). They are unrelated to this fix and are tracked as a separate AMM-compile-blocker issue.
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
