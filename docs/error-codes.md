# AMM Contract Error Code Reference

This document provides a complete reference for every error code emitted by the
Soroban AMM contracts.  For each code you will find:

- **Numeric code** – the on-chain discriminant embedded in the XDR
  `InvokeHostFunctionResult` when the contract returns that error.
- **Cause** – what precondition was violated.
- **Remedy** – how a caller should recover.

Use the numeric code when parsing RPC responses or writing off-chain tooling.
The symbolic name is what appears in Rust source and in decoded XDR.

---

## AmmPool (`contracts/amm`)

Defined in [contracts/amm/src/lib.rs](../contracts/amm/src/lib.rs) as `AmmError`.

| Code | Symbol | Cause | Remedy |
|------|--------|-------|--------|
| 1 | `AlreadyInitialized` | `initialize` (or `initialize_with_flash_loan_fee`) was called on a pool that has already been set up. The presence of `DataKey::TokenA` in instance storage is the guard. | Deploy a fresh pool contract instead of re-initializing. |
| 2 | `InvalidFeeBps` | A fee value was outside `[0, 10 000]` bps, or `protocol_fee_bps` exceeded `fee_bps`, or `new_fee_bps` was below the current `protocol_fee_bps`. | Ensure `0 ≤ protocol_fee_bps ≤ fee_bps ≤ 10 000`. |
| 3 | `InsufficientShares` | The LP share burn amount in `remove_liquidity` exceeded the provider's actual LP token balance. | Query `shares_of(provider)` first and cap the burn amount to that value. |
| 4 | `DeadlineExceeded` | The `deadline` ledger timestamp passed before the transaction was included. | Re-submit with `deadline = current_ledger_timestamp + buffer`. |
| 5 | `SlippageExceeded` | For swaps: `amount_out < min_out` or `required_in > max_in`. For `add_liquidity`: minted shares < `min_shares`. For `remove_liquidity`: `out_a < min_a` or `out_b < min_b`. | Widen slippage bounds or use `simulate_swap` / `get_amount_in` to recalculate before submitting. |
| 6 | `Paused` | The pool has been administratively paused via `pause()`. All state-mutating functions (swap, add/remove liquidity, flash loan) reject calls while paused. | Wait for the admin (or governance) to call `unpause()`. |
| 7 | `Unauthorized` | A caller passed an `admin` address that does not match the stored admin. | Use the correct admin keypair. The current admin can be read with `get_info().admin`. |
| 8 | `ZeroAmount` | An `amount_*` argument was zero or negative. | Pass a strictly positive value. |
| 9 | `InvalidToken` | `token_in`, `token_out`, or `token` did not match either of the two pool tokens. | Use `get_info()` to discover valid token addresses. |
| 10 | `EmptyPool` | A swap or `price_ratio` was attempted on a pool where at least one reserve is zero. | Add liquidity before trading. |
| 11 | `InsufficientLiquidity` | Either `amount_out ≥ reserve_out` (would drain the pool), or `reserve < amount` for a flash loan, or the flash loan receiver did not repay (`balance_after < balance_before + fee`). | Reduce the trade/loan size, or ensure the flash loan receiver repays the principal plus fee within the callback. |
| 12 | `NoPendingAdmin` | `accept_admin` was called when no admin transfer is in progress. | Call `propose_admin` first to nominate a successor. |
| 13 | `WrongAdmin` | `accept_admin` was called by an address that does not match the pending nominee. | Have the correct address (the one passed to `propose_admin`) call `accept_admin`. |
| 14 | `Reentrant` | A flash loan receiver attempted to call back into `swap`, `add_liquidity`, `remove_liquidity`, or `flash_loan` on the same pool while the reentrancy lock (`DataKey::Locked`) was held. | Do not call pool functions from inside `on_flash_loan`. Perform all swaps and liquidity operations *before* or *after* the flash loan, not during the callback. |
| 15 | `CircuitBreaker` | The spot price deviated more than the configured threshold (default 5 000 bps = 50 %) from the value at the start of the block. The pool has been automatically paused. | Wait for the cooldown period to elapse (default 600 s) and call `try_circuit_breaker_recovery`, or have governance call `unpause`. |

---

## ConcentratedLiquidity (`contracts/concentrated_liquidity`)

Defined in [contracts/concentrated_liquidity/src/lib.rs](../contracts/concentrated_liquidity/src/lib.rs) as `ClError`.

| Code | Symbol | Cause | Remedy |
|------|--------|-------|--------|
| 1 | `AlreadyInitialized` | `initialize` called on an already-configured pool. | Deploy a new pool. |
| 2 | `TokensMustDiffer` | `token_a == token_b` during initialization. | Use two distinct token addresses. |
| 3 | `InvalidFeeBps` | Fee outside `[0, 10 000]` bps. | Use a value in the accepted range. |
| 4 | `TickOutOfRange` | A tick value was outside `[-MAX_TICK, MAX_TICK]` (±887 272). `lower_tick` must also be < `upper_tick`. | Keep ticks inside the valid range and ensure lower < upper. |
| 5 | `ZeroAmounts` | Both `amount_a_desired` and `amount_b_desired` were zero or negative. | Provide at least one positive desired amount. |
| 6 | `SlippageExceeded` | `amount_a < min_a` or `amount_b < min_b` on `mint_position`, or output below `min_out` on `swap`. | Widen slippage bounds. |
| 7 | `ZeroLiquidity` | Computed liquidity for a new position was zero (amounts too small relative to the price range). | Increase the deposit amounts or narrow the tick range. |
| 8 | `InsufficientLiquidity` | `burn_position` requested more liquidity than the position holds, or swap output would exceed available liquidity. | Burn ≤ `position.liquidity`; reduce swap size. |
| 9 | `PositionNotFound` | `collect_fees`, `burn_position`, or `collect_all` referenced a position `(owner, lower_tick, upper_tick)` that does not exist. | Verify the position exists with `get_position` before operating on it. |
| 10 | `DeadlineExpired` | The `deadline` timestamp passed before execution. | Re-submit with a future deadline. |
| 11 | `Paused` | Pool is paused. | Wait for admin to unpause. |
| 12 | `Unauthorized` | Admin mismatch. | Use the stored admin address. |
| 13 | `TickNotAligned` | A tick was not a multiple of `tick_spacing`. | Round ticks to the nearest multiple of `tick_spacing`. |
| 14 | `InvalidTickSpacing` | `tick_spacing ≤ 0` during initialization. | Use a positive tick spacing (common values: 1, 10, 60, 200). |
| 15 | `TickNotInitialized` | A swap crossed into a tick that has no liquidity (never been used by any position). | Ensure positions cover the full swap range, or use a smaller swap amount. |
| 16 | `InvalidToken` | `token_in` is not `token_a` or `token_b`. | Check `get_info()` for valid token addresses. |

---

## Factory (`contracts/factory`)

Defined in [contracts/factory/src/lib.rs](../contracts/factory/src/lib.rs) as `FactoryError`.

| Code | Symbol | Cause | Remedy |
|------|--------|-------|--------|
| 1 | `AlreadyInitialized` | `initialize` called twice. | Only initialize the factory once after deployment. |
| 2 | `InvalidFeeBps` | Fee outside `[0, 10 000]`. | Correct the fee value. |
| 3 | `PoolAlreadyExists` | `create_pool` was called for a `(token_a, token_b)` pair that already has an AMM pool. | Use `get_pool` to retrieve the existing pool address. |
| 4 | `ClPoolAlreadyExists` | `create_cl_pool` was called for a pair/fee that already has a CL pool. | Use `get_cl_pool` to retrieve the existing address. |
| 5 | `ClWasmNotSet` | Tried to create a CL pool before the CL WASM hash was registered via `set_cl_wasm`. | Call `set_cl_wasm` with the uploaded CL contract hash first. |
| 6 | `Unauthorized` | Non-admin called an admin-only factory function. | Use the factory admin keypair. |

---

## ClPositionNft (`contracts/cl_position_nft`)

Defined in [contracts/cl_position_nft/src/lib.rs](../contracts/cl_position_nft/src/lib.rs) as `NftError`.

| Code | Symbol | Cause | Remedy |
|------|--------|-------|--------|
| 1 | `AlreadyInitialized` | `initialize` called twice. | Deploy a new NFT contract instead. |
| 2 | `Unauthorized` | A caller other than the registered `cl_pool` called `mint`/`burn`, or `set_ttl_params`/`migrate_ownership_index` was called by a non-admin (or before `initialize`). | Use the registered pool address (for mint/burn) or the admin address (for the others). |
| 3 | `TokenNotFound` | `owner_of`, `position_meta`, `burn`, `approve`, or `transfer` referenced a `token_id` that does not exist (never minted, or already burned). | Verify the id with `owner_of`/`total_supply` first. |
| 4 | `NotOwnerOrApproved` | `transfer` called by an address that is neither the token's owner, its approved address, nor an approved operator for the owner. | Use the owner, an approved address, or an approved operator. |
| 5 | `InvalidReceiver` | Reserved; not currently returned by any function. | — |
| 6 | `InvalidTtlConfig` | `set_ttl_params` was called with `bump_to < min_ttl_threshold`. | Ensure `bump_to >= min_ttl_threshold`. |
| 7 | `TooManyPositions` | `mint` (or the `to` side of `transfer`) would push an owner's O(1)-indexed holdings past `MAX_POSITIONS_PER_OWNER` (10,000). Defence in depth introduced in #697, on top of the O(1) index fix itself. | Transfer or burn existing positions for that owner first. |

#697 replaced the single unbounded `OwnedTokens(owner) -> Vec<u64>` per-owner
list with a constant-cost index (`OwnerTokenCount`, `OwnerTokenByIndex`,
`TokenIndexOfOwner`), so `mint`, `burn`, and `transfer` each touch a fixed
number of storage entries regardless of how many positions an owner holds.
`balance_of`, `tokens_of`, `tokens_of_paginated`, and
`token_of_owner_by_index` read only that index. A legacy `OwnedTokens`
vector from before this upgrade is not written to again — `burn` and the
"from" side of `transfer` fall back to an O(n) removal from it only when a
token has no index slot, and the admin-only `migrate_ownership_index` moves
a legacy vector into the index in bounded chunks. See the "#697: ownership
index" doc comment in `cl_position_nft/src/lib.rs` for the full design and
its trade-offs.

---

## Governance (`contracts/governance`)

Defined in [contracts/governance/src/lib.rs](../contracts/governance/src/lib.rs) as `GovernanceError`.

| Code | Symbol | Cause | Remedy |
|------|--------|-------|--------|
| 1 | `AlreadyInitialized` | Governance initialized twice. | Deploy a new governance contract. |
| 2 | `InvalidVotingPeriod` | Voting period is zero or below the minimum. | Use a period ≥ 1 ledger. |
| 3 | `InvalidTimelock` | Timelock duration is below the minimum. | Increase the timelock value. |
| 4 | `InvalidQuorumBps` | Quorum outside `(0, 10 000]`. | Use a positive bps value ≤ 10 000. |
| 5 | `InvalidProposerStake` | Proposer stake threshold is zero. | Set a positive minimum stake. |
| 6 | `InvalidFeeBps` | Fee value out of range. | Use `[0, 10 000]`. |
| 7 | `ZeroTotalSupply` | Vote weight computed on a zero LP supply. | Seed the pool with liquidity before creating proposals. |
| 8 | `InsufficientStake` | Proposer's LP stake is below `min_proposer_stake`. | Acquire more LP tokens before proposing. |
| 9 | `ProposalNotFound` | Referenced proposal ID does not exist. | Use `get_proposal` to verify the ID. |
| 10 | `VotingNotStarted` | Tried to vote before the proposal's start block. | Wait until the voting period begins. |
| 11 | `VotingPeriodEnded` | Tried to vote after the voting period closed. | Votes cannot be cast after closure. |
| 12 | `AlreadyExecuted` | `execute` called on a proposal that was already executed. | Each proposal can only be executed once. |
| 13 | `ProposalCancelled` | Action on a cancelled proposal. | The proposal is terminal; create a new one. |
| 14 | `AlreadyVoted` | The caller already cast a vote on this proposal. | Each address can vote once per proposal. |
| 15 | `NoVotingPower` | Caller's LP balance snapshot at proposal creation was zero. | You must hold LP tokens at the proposal creation block to vote. |
| 16 | `VotingPeriodActive` | Tried to execute while voting is still open. | Wait for the voting period to end. |
| 17 | `ProposalExpired` | `execute` called after the execution window expired. | Create a new proposal. |
| 18 | `TimelockNotElapsed` | `execute` called before the timelock delay elapsed. | Wait the full timelock duration after the voting period ends. |
| 19 | `QuorumNotMet` | Total votes did not reach the quorum threshold. | The proposal fails; if needed, create a new one with broader participation. |
| 20 | `ProposalDefeated` | More votes were cast against than for. | Create a new proposal with updated parameters. |
| 21 | `NotProposer` | `cancel` called by someone other than the original proposer. | Only the proposer can cancel before voting ends. |
| 22 | `NoLockedVote` | `unlock_vote` called when no vote was locked. | Only addresses that voted with token lock need to call `unlock_vote`. |
| 23 | `ProposalNotConcluded` | `unlock_vote` or `claim_rewards` called before the proposal concluded. | Wait for the proposal to reach a terminal state. |
| 24 | `CannotDelegateToSelf` | A delegator tried to delegate to themselves. | Delegate to a different address. |
| 25 | `Unauthorized` | Admin-only operation called by non-admin. | Use the governance admin. |
| 26 | `HasDelegated` | Operation requires direct voting power but caller has already delegated. | Undelegate first. |
| 27 | `DelegationCycle` | The delegation would create a cycle (A → B → … → A). | Choose a delegate that is not already part of this principal's delegation chain. |
| 28 | `ProposalVetoed` | A veto multisig vetoed the proposal. | Create a new proposal; adjust to address the veto reason. |
| 29 | `VetoWindowExpired` | Veto attempted after the veto window closed. | Vetoes must be cast within the veto window after voting ends. |
| 30 | `NotVetoMultisig` | Veto called by an address that is not the configured veto multisig. | Only the veto multisig can veto proposals. |
| 31 | `InsufficientSnapshotBal` | Snapshot balance at proposal creation was insufficient. | Acquire more LP tokens before the proposal snapshot block. |
| 32 | `VetoMultisigNotSet` | Veto-related operation called when no veto multisig is configured. | Configure the veto multisig during governance initialization. |

---

## Decoding errors from RPC responses

When `stellar-sdk-rs` (or the Soroban RPC) returns a failed invocation, the
result contains an XDR `ScError` with kind `Contract` and a `code` field. Map
the code to the table above using the contract address to identify which enum
applies.

```rust
// Pseudocode — adapt to your stellar-sdk-rs version
match result {
    InvocationResult::Err(ScError::Contract(code)) => {
        match contract_address {
            addr if addr == amm_pool => AmmError::from(code),
            addr if addr == cl_pool  => ClError::from(code),
            addr if addr == factory  => FactoryError::from(code),
            _ => eprintln!("unknown contract error {code}"),
        }
    }
    _ => {}
}
```

All error codes are stable across minor contract upgrades. A code is only
ever removed or renumbered in a major version bump accompanied by a migration
guide in [CHANGELOG.md](../CHANGELOG.md).

---

## Keeping documentation in sync

Error code definitions live alongside the contract source.  When adding a new
error variant:

1. Add the variant to the `contracterror` enum in the relevant `lib.rs`.
2. Add a row to the matching table in this file.
3. Update `contracts/amm-sdk/src/types.rs` (`SdkAmmError`) if the change
   affects the AMM pool contract.

CI enforces this via a lint that counts enum variants and compares against
the row count in this document (see `.github/workflows/error-doc-check.yml`).
