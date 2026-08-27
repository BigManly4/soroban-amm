#![no_std]

//! TWAL (time-weighted average liquidity) consumer contract.

use soroban_sdk::{
    contract, contractclient, contracterror, contractimpl, contracttype, Address, Env, Symbol, Vec,
};

#[contractclient(name = "AmmPoolLiquidityClient")]
pub trait AmmPoolLiquidityOracle {
    fn get_liquidity_cumulative(env: Env) -> (i128, u64);
}

#[contractclient(name = "ClPoolLiquidityClient")]
pub trait ClPoolLiquidityOracle {
    fn active_liquidity(env: Env) -> i128;
    fn get_tick_cumulative(env: Env) -> (i64, u64);
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum TwalError {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    ZeroWindow = 3,
    InsufficientHistory = 4,
    NoSnapshotFound = 5,
    ElapsedZero = 6,
    /// `add_tracked_pool` (explicit or implicit, via `save_snapshot`/
    /// `save_cl_snapshot`) would push the tracked set past
    /// `TwalConsumer::MAX_TRACKED_POOLS`.
    TooManyTrackedPools = 7,
    /// `remove_tracked_pool` was called with a pool that is not in the
    /// tracked set.
    NotTracked = 8,
    /// `window_seconds` exceeded `TwalConsumer::MAX_WINDOW_SECONDS`.
    WindowTooLarge = 9,
    /// `get_twal_batch` was called with more pools than
    /// `TwalConsumer::MAX_TRACKED_POOLS`.
    TooManyPools = 10,
    /// The cross-contract call into the pool's liquidity oracle failed —
    /// either it returned a host-level invocation error, or the callee
    /// panicked (e.g. the address is not a contract, or has a bug).
    CrossContractCallFailed = 11,
}

#[contracttype]
pub enum DataKey {
    Keeper,
    LiquiditySnapshot(Address, u64),
    TrackedPoolsPersistent,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LiquiditySnapshot {
    pub cum_liquidity: i128,
    pub pool_ts: u64,
}

/// One pool's result from a fault-isolated batch TWAL read.
///
/// `get_twal_all_safe` and `get_twal_batch` always return exactly one entry
/// per pool queried, whether or not that pool's read succeeded, so a single
/// dead or reverting pool can never take down the rest of the batch.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct TwalEntry {
    pub pool: Address,
    pub twal: i128,
    pub ok: bool,
    /// Populated when `ok == false`; the `TwalError` discriminant that
    /// caused this pool to be skipped. Meaningless (`0`) when `ok == true`.
    pub error_code: u32,
}

#[contract]
pub struct TwalConsumer;

#[contractimpl]
impl TwalConsumer {
    pub const SNAPSHOT_TTL_LEDGERS: u32 = 120_960;

    /// Ceiling on the tracked-pool set. Keeper-only mutation plus this cap
    /// bounds `get_twal_all` / `get_twal_all_safe` cost so neither read path
    /// can be pushed past the transaction resource limit by an oversized
    /// tracked set.
    pub const MAX_TRACKED_POOLS: u32 = 100;

    /// Ceiling on `window_seconds` for any TWAL read (90 days). Bounds how
    /// far back a snapshot lookup can reach and keeps a single call's cost
    /// predictable regardless of caller-supplied input.
    pub const MAX_WINDOW_SECONDS: u64 = 7_776_000;

    pub fn initialize(env: Env, keeper: Address) -> Result<(), TwalError> {
        if env.storage().instance().has(&DataKey::Keeper) {
            return Err(TwalError::AlreadyInitialized);
        }
        env.storage().instance().set(&DataKey::Keeper, &keeper);
        Ok(())
    }

    pub fn get_keeper(env: Env) -> Result<Address, TwalError> {
        env.storage()
            .instance()
            .get(&DataKey::Keeper)
            .ok_or(TwalError::NotInitialized)
    }

    fn require_keeper(env: &Env) -> Result<(), TwalError> {
        Self::get_keeper(env.clone())?.require_auth();
        Ok(())
    }

    pub fn save_snapshot(env: Env, pool: Address) -> Result<(), TwalError> {
        Self::require_keeper(&env)?;
        let (cum, pool_ts) = AmmPoolLiquidityClient::new(&env, &pool).get_liquidity_cumulative();
        let ledger_ts = env.ledger().timestamp();
        let snapshot = LiquiditySnapshot {
            cum_liquidity: cum,
            pool_ts,
        };
        let key = DataKey::LiquiditySnapshot(pool.clone(), ledger_ts);
        env.storage().persistent().set(&key, &snapshot);
        env.storage().persistent().extend_ttl(
            &key,
            Self::SNAPSHOT_TTL_LEDGERS / 2,
            Self::SNAPSHOT_TTL_LEDGERS,
        );
        Self::add_tracked_pool_internal(&env, &pool)?;
        Ok(())
    }

    fn load_tracked(env: &Env) -> Vec<Address> {
        env.storage()
            .persistent()
            .get(&DataKey::TrackedPoolsPersistent)
            .unwrap_or_else(|| Vec::new(env))
    }

    fn store_tracked(env: &Env, tracked: &Vec<Address>) {
        env.storage()
            .persistent()
            .set(&DataKey::TrackedPoolsPersistent, tracked);
        env.storage().persistent().extend_ttl(
            &DataKey::TrackedPoolsPersistent,
            Self::SNAPSHOT_TTL_LEDGERS / 2,
            Self::SNAPSHOT_TTL_LEDGERS,
        );
    }

    /// Shared implementation behind both the explicit `add_tracked_pool` and
    /// the implicit registration inside `save_snapshot` / `save_cl_snapshot`.
    /// Idempotent: adding an already-tracked pool is a no-op `Ok(())` and
    /// emits no event. Rejects growth past `MAX_TRACKED_POOLS`.
    fn add_tracked_pool_internal(env: &Env, pool: &Address) -> Result<(), TwalError> {
        let mut tracked = Self::load_tracked(env);
        if tracked.first_index_of(pool.clone()).is_some() {
            return Ok(());
        }
        if tracked.len() >= Self::MAX_TRACKED_POOLS {
            return Err(TwalError::TooManyTrackedPools);
        }
        tracked.push_back(pool.clone());
        Self::store_tracked(env, &tracked);
        env.events()
            .publish((Symbol::new(env, "pool_add"),), pool.clone());
        Ok(())
    }

    /// Adds `pool` to the tracked set. Keeper-only, idempotent, rejects
    /// growth past `MAX_TRACKED_POOLS`.
    pub fn add_tracked_pool(env: Env, pool: Address) -> Result<(), TwalError> {
        Self::require_keeper(&env)?;
        Self::add_tracked_pool_internal(&env, &pool)
    }

    /// Removes `pool` from the tracked set, preserving the relative order of
    /// the remaining pools. Keeper-only. Errors with `NotTracked` if the pool
    /// was never in the set.
    pub fn remove_tracked_pool(env: Env, pool: Address) -> Result<(), TwalError> {
        Self::require_keeper(&env)?;
        let mut tracked = Self::load_tracked(&env);
        let idx = tracked
            .first_index_of(pool.clone())
            .ok_or(TwalError::NotTracked)?;
        tracked.remove(idx);
        Self::store_tracked(&env, &tracked);
        env.events()
            .publish((Symbol::new(&env, "pool_remove"),), pool);
        Ok(())
    }

    /// Returns whether `pool` is currently in the tracked set.
    pub fn is_tracked(env: Env, pool: Address) -> bool {
        Self::load_tracked(&env).first_index_of(pool).is_some()
    }

    /// Returns the number of currently tracked pools.
    pub fn get_tracked_pool_count(env: Env) -> u32 {
        Self::load_tracked(&env).len()
    }

    /// Returns up to `limit` tracked pool addresses starting at `offset`.
    /// `limit == 0` and `offset >= count` both return an empty `Vec`.
    pub fn get_tracked_pools_paginated(env: Env, offset: u32, limit: u32) -> Vec<Address> {
        let tracked = Self::load_tracked(&env);
        let len = tracked.len();
        if limit == 0 || offset >= len {
            return Vec::new(&env);
        }
        let end = core::cmp::min(offset.saturating_add(limit), len);
        tracked.slice(offset..end)
    }

    /// Computes a single pool's TWAL entry without ever panicking or
    /// propagating a typed error — used by both `get_twal_all_safe` and
    /// `get_twal_batch` so a single bad pool can never abort a batch read.
    /// The cross-contract call uses `try_invoke_contract` (via the generated
    /// `try_*` client method) so a panicking or non-contract callee is
    /// caught here too, not just a typed `Err`.
    fn compute_twal_entry(env: &Env, pool: Address, window_seconds: u64) -> TwalEntry {
        let fail = |code: TwalError| TwalEntry {
            pool: pool.clone(),
            twal: 0,
            ok: false,
            error_code: code as u32,
        };

        if window_seconds == 0 {
            return fail(TwalError::ZeroWindow);
        }
        if window_seconds > Self::MAX_WINDOW_SECONDS {
            return fail(TwalError::WindowTooLarge);
        }

        let (cum_now, pool_ts_now) =
            match AmmPoolLiquidityClient::new(env, &pool).try_get_liquidity_cumulative() {
                Ok(Ok(v)) => v,
                _ => return fail(TwalError::CrossContractCallFailed),
            };

        let ledger_ts_now = env.ledger().timestamp();
        if ledger_ts_now < window_seconds {
            return fail(TwalError::InsufficientHistory);
        }
        let then_ts = ledger_ts_now - window_seconds;
        let snapshot: LiquiditySnapshot = match env
            .storage()
            .persistent()
            .get(&DataKey::LiquiditySnapshot(pool.clone(), then_ts))
        {
            Some(s) => s,
            None => return fail(TwalError::NoSnapshotFound),
        };

        let delta = (cum_now as u128).wrapping_sub(snapshot.cum_liquidity as u128) as i128;
        let elapsed = (pool_ts_now - snapshot.pool_ts) as i128;
        if elapsed <= 0 {
            return fail(TwalError::ElapsedZero);
        }
        TwalEntry {
            pool,
            twal: delta / elapsed,
            ok: true,
            error_code: 0,
        }
    }

    fn error_from_code(code: u32) -> TwalError {
        match code {
            3 => TwalError::ZeroWindow,
            4 => TwalError::InsufficientHistory,
            5 => TwalError::NoSnapshotFound,
            6 => TwalError::ElapsedZero,
            9 => TwalError::WindowTooLarge,
            _ => TwalError::CrossContractCallFailed,
        }
    }

    pub fn get_twal_liquidity(
        env: Env,
        pool: Address,
        window_seconds: u64,
    ) -> Result<i128, TwalError> {
        if window_seconds == 0 {
            return Err(TwalError::ZeroWindow);
        }
        if window_seconds > Self::MAX_WINDOW_SECONDS {
            return Err(TwalError::WindowTooLarge);
        }
        let (cum_now, pool_ts_now) =
            AmmPoolLiquidityClient::new(&env, &pool).get_liquidity_cumulative();
        let ledger_ts_now = env.ledger().timestamp();
        if ledger_ts_now < window_seconds {
            return Err(TwalError::InsufficientHistory);
        }
        let then_ts = ledger_ts_now - window_seconds;
        let snapshot: LiquiditySnapshot = env
            .storage()
            .persistent()
            .get(&DataKey::LiquiditySnapshot(pool, then_ts))
            .ok_or(TwalError::NoSnapshotFound)?;

        let delta = (cum_now as u128).wrapping_sub(snapshot.cum_liquidity as u128) as i128;
        let elapsed = (pool_ts_now - snapshot.pool_ts) as i128;
        if elapsed <= 0 {
            return Err(TwalError::ElapsedZero);
        }
        Ok(delta / elapsed)
    }

    /// Fault-isolated batch read over every currently tracked pool. Returns
    /// exactly one `TwalEntry` per tracked pool, always — a pool with no
    /// usable snapshot, a non-contract address, or a panicking callee yields
    /// `ok: false` in its slot rather than aborting the call.
    pub fn get_twal_all_safe(env: Env, window_seconds: u64) -> Vec<TwalEntry> {
        let tracked = Self::load_tracked(&env);
        let mut out = Vec::new(&env);
        for i in 0..tracked.len() {
            let pool = tracked.get(i).unwrap();
            out.push_back(Self::compute_twal_entry(&env, pool, window_seconds));
        }
        out
    }

    /// Fault-isolated batch read over a caller-supplied subset of pools, so
    /// a consumer can read the pools it cares about without depending on the
    /// global tracked set. Capped at `MAX_TRACKED_POOLS` entries.
    pub fn get_twal_batch(
        env: Env,
        pools: Vec<Address>,
        window_seconds: u64,
    ) -> Result<Vec<TwalEntry>, TwalError> {
        if pools.len() > Self::MAX_TRACKED_POOLS {
            return Err(TwalError::TooManyPools);
        }
        let mut out = Vec::new(&env);
        for i in 0..pools.len() {
            out.push_back(Self::compute_twal_entry(
                &env,
                pools.get(i).unwrap(),
                window_seconds,
            ));
        }
        Ok(out)
    }

    /// Batch TWAL read over every tracked pool. Kept on the ABI for existing
    /// callers: returns `Ok` with the same values as before when every pool
    /// succeeds, and the first `Err` encountered otherwise — identical
    /// external behaviour to the pre-#695 implementation. New callers should
    /// prefer `get_twal_all_safe` for per-pool detail.
    pub fn get_twal_all(env: Env, window_seconds: u64) -> Result<Vec<(Address, i128)>, TwalError> {
        let entries = Self::get_twal_all_safe(env.clone(), window_seconds);
        let mut results: Vec<(Address, i128)> = Vec::new(&env);
        for i in 0..entries.len() {
            let entry = entries.get(i).unwrap();
            if !entry.ok {
                return Err(Self::error_from_code(entry.error_code));
            }
            results.push_back((entry.pool, entry.twal));
        }
        Ok(results)
    }

    pub fn get_tracked_pools(env: Env) -> Vec<Address> {
        Self::load_tracked(&env)
    }

    pub fn save_cl_snapshot(env: Env, pool: Address) -> Result<(), TwalError> {
        Self::require_keeper(&env)?;
        let active = ClPoolLiquidityClient::new(&env, &pool).active_liquidity();
        let (_tick_cum, pool_ts) = ClPoolLiquidityClient::new(&env, &pool).get_tick_cumulative();
        let ledger_ts = env.ledger().timestamp();
        let snapshot = LiquiditySnapshot {
            cum_liquidity: active,
            pool_ts,
        };
        let key = DataKey::LiquiditySnapshot(pool.clone(), ledger_ts);
        env.storage().persistent().set(&key, &snapshot);
        env.storage().persistent().extend_ttl(
            &key,
            Self::SNAPSHOT_TTL_LEDGERS / 2,
            Self::SNAPSHOT_TTL_LEDGERS,
        );
        Self::add_tracked_pool_internal(&env, &pool)?;
        Ok(())
    }

    /// Deletes a stored liquidity snapshot from persistent storage. Keeper-only.
    pub fn delete_snapshot(env: Env, pool: Address, ledger_ts: u64) -> Result<(), TwalError> {
        Self::require_keeper(&env)?;
        let key = DataKey::LiquiditySnapshot(pool.clone(), ledger_ts);
        if !env.storage().persistent().has(&key) {
            return Err(TwalError::NoSnapshotFound);
        }
        env.storage().persistent().remove(&key);
        env.events()
            .publish((Symbol::new(&env, "snapshot_deleted"),), (pool, ledger_ts));
        Ok(())
    }
    /// Returns the time‑weighted average active liquidity for a CL pool over `window_seconds`.
    pub fn get_cl_twal(env: Env, pool: Address, window_seconds: u64) -> i128 {
        assert!(window_seconds > 0, "window_seconds must be > 0");

        // Current active liquidity and pool timestamp
        let active = ClPoolLiquidityClient::new(&env, &pool).active_liquidity();
        let (_, pool_ts_now) = ClPoolLiquidityClient::new(&env, &pool).get_tick_cumulative();

        let ledger_ts_now = env.ledger().timestamp();
        assert!(
            ledger_ts_now >= window_seconds,
            "ledger timestamp is smaller than requested window"
        );

        let then_ts = ledger_ts_now - window_seconds;
        let snapshot: LiquiditySnapshot = env
            .storage()
            .persistent()
            .get(&DataKey::LiquiditySnapshot(pool, then_ts))
            .unwrap_or_else(|| panic!("missing liquidity snapshot at {then_ts}"));

        let delta = (active as u128).wrapping_sub(snapshot.cum_liquidity as u128) as i128;
        let elapsed = (pool_ts_now - snapshot.pool_ts) as i128;
        assert!(elapsed > 0, "window too small (pool time did not advance)");
        delta / elapsed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use amm::{AmmPool, AmmPoolClient};
    use soroban_sdk::{
        testutils::{Address as _, Ledger},
        token::{StellarAssetClient, TokenClient as StellarTokenClient},
        Address, Env,
    };
    use token::LpToken;

    fn create_sac<'a>(
        env: &'a Env,
        admin: &Address,
    ) -> (StellarTokenClient<'a>, StellarAssetClient<'a>) {
        let contract = env.register_stellar_asset_contract_v2(admin.clone());
        (
            StellarTokenClient::new(env, &contract.address()),
            StellarAssetClient::new(env, &contract.address()),
        )
    }

    #[test]
    fn test_twal_increases_with_liquidity_and_time() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(10_000);

        let admin = Address::generate(&env);
        let amm_addr = env.register_contract(None, AmmPool);
        let lp_addr = env.register_contract(None, LpToken);
        let consumer_addr = env.register_contract(None, TwalConsumer);

        token::LpTokenClient::new(&env, &lp_addr).initialize(
            &amm_addr,
            &soroban_sdk::String::from_str(&env, "LP"),
            &soroban_sdk::String::from_str(&env, "LP"),
            &7u32,
        );

        let (ta, ta_sac) = create_sac(&env, &admin);
        let (tb, tb_sac) = create_sac(&env, &admin);
        AmmPoolClient::new(&env, &amm_addr).initialize(
            &admin,
            &ta.address,
            &tb.address,
            &lp_addr,
            &30_i128,
            &admin,
            &0_i128,
        );

        let provider = Address::generate(&env);
        ta_sac.mint(&provider, &1_000_000_i128);
        tb_sac.mint(&provider, &1_000_000_i128);
        AmmPoolClient::new(&env, &amm_addr).add_liquidity(
            &provider,
            &1_000_000_i128,
            &1_000_000_i128,
            &0_i128,
            &u64::MAX,
        );

        let consumer = TwalConsumerClient::new(&env, &consumer_addr);
        consumer.initialize(&admin);
        consumer.save_snapshot(&amm_addr);

        env.ledger().with_mut(|l| l.timestamp = 10_600);
        ta_sac.mint(&provider, &200_000_i128);
        tb_sac.mint(&provider, &200_000_i128);
        AmmPoolClient::new(&env, &amm_addr).add_liquidity(
            &provider,
            &100_000_i128,
            &100_000_i128,
            &0_i128,
            &u64::MAX,
        );
        consumer.save_snapshot(&amm_addr);

        env.ledger().with_mut(|l| l.timestamp = 11_200);
        let trader = Address::generate(&env);
        ta_sac.mint(&trader, &1_000_i128);
        AmmPoolClient::new(&env, &amm_addr).swap(
            &trader,
            &ta.address,
            &1_000_i128,
            &0_i128,
            &u64::MAX,
        );

        let twal = consumer.get_twal_liquidity(&amm_addr, &600);
        assert!(twal > 0);
    }

    #[test]
    fn test_zero_window_returns_error() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(10_000);

        let admin = Address::generate(&env);
        let pool = Address::generate(&env);
        let consumer_addr = env.register_contract(None, TwalConsumer);
        let consumer = TwalConsumerClient::new(&env, &consumer_addr);
        consumer.initialize(&admin);

        assert_eq!(
            consumer.try_get_twal_liquidity(&pool, &0_u64),
            Err(Ok(TwalError::ZeroWindow))
        );
    }

    #[test]
    fn test_save_snapshot_requires_keeper_auth() {
        let env = Env::default();
        env.ledger().set_timestamp(10_000);

        let keeper = Address::generate(&env);
        let pool = Address::generate(&env);
        let consumer_addr = env.register_contract(None, TwalConsumer);
        let consumer = TwalConsumerClient::new(&env, &consumer_addr);
        consumer.initialize(&keeper);

        assert!(consumer.try_save_snapshot(&pool).is_err());
        assert!(consumer.try_save_cl_snapshot(&pool).is_err());
    }

    #[test]
    fn test_save_snapshot_fails_when_uninitialized() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(10_000);

        let pool = Address::generate(&env);
        let consumer_addr = env.register_contract(None, TwalConsumer);
        let consumer = TwalConsumerClient::new(&env, &consumer_addr);

        assert!(consumer.try_save_snapshot(&pool).is_err());
    }

    #[test]
    fn test_initialize_is_idempotent_guard() {
        let env = Env::default();
        env.mock_all_auths();

        let keeper = Address::generate(&env);
        let consumer_addr = env.register_contract(None, TwalConsumer);
        let consumer = TwalConsumerClient::new(&env, &consumer_addr);

        consumer.initialize(&keeper);
        assert_eq!(consumer.get_keeper(), keeper);
        assert_eq!(
            consumer.try_initialize(&Address::generate(&env)),
            Err(Ok(TwalError::AlreadyInitialized))
        );
    }

    #[test]
    fn test_delete_snapshot_removes_liquidity_snapshot() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(10_000);

        let admin = Address::generate(&env);
        let amm_addr = env.register_contract(None, AmmPool);
        let lp_addr = env.register_contract(None, LpToken);
        let consumer_addr = env.register_contract(None, TwalConsumer);

        token::LpTokenClient::new(&env, &lp_addr).initialize(
            &amm_addr,
            &soroban_sdk::String::from_str(&env, "LP"),
            &soroban_sdk::String::from_str(&env, "LP"),
            &7u32,
        );

        let (ta, ta_sac) = create_sac(&env, &admin);
        let (tb, tb_sac) = create_sac(&env, &admin);
        AmmPoolClient::new(&env, &amm_addr).initialize(
            &admin,
            &ta.address,
            &tb.address,
            &lp_addr,
            &30_i128,
            &admin,
            &0_i128,
        );

        let provider = Address::generate(&env);
        ta_sac.mint(&provider, &1_000_000_i128);
        tb_sac.mint(&provider, &1_000_000_i128);
        AmmPoolClient::new(&env, &amm_addr).add_liquidity(
            &provider,
            &1_000_000_i128,
            &1_000_000_i128,
            &0_i128,
            &u64::MAX,
        );

        let consumer = TwalConsumerClient::new(&env, &consumer_addr);
        consumer.initialize(&admin);
        consumer.save_snapshot(&amm_addr);

        // Snapshot stored at ledger timestamp 10_000; deleting it removes it so a
        // later TWAL query over that window can no longer find it.
        consumer.delete_snapshot(&amm_addr, &10_000_u64);

        env.ledger().with_mut(|l| l.timestamp = 10_600);
        assert_eq!(
            consumer.try_get_twal_liquidity(&amm_addr, &600),
            Err(Ok(TwalError::NoSnapshotFound))
        );
    }

    #[test]
    fn test_delete_snapshot_requires_keeper_auth() {
        let env = Env::default();
        env.ledger().set_timestamp(10_000);

        let keeper = Address::generate(&env);
        let pool = Address::generate(&env);
        let consumer_addr = env.register_contract(None, TwalConsumer);
        let consumer = TwalConsumerClient::new(&env, &consumer_addr);
        consumer.initialize(&keeper);

        assert!(consumer.try_delete_snapshot(&pool, &10_000_u64).is_err());
    }

    // ---- #695: tracked-pool lifecycle & fault-isolated batch reads ----

    /// A stub pool contract whose liquidity oracle always panics. Used to
    /// prove that batch reads catch a genuinely trapping callee, not only a
    /// typed `TwalError`.
    #[contract]
    pub struct PanicPool;

    #[contractimpl]
    impl PanicPool {
        pub fn get_liquidity_cumulative(_env: Env) -> (i128, u64) {
            panic!("PanicPool always panics");
        }
    }

    fn setup_consumer(env: &Env) -> (Address, TwalConsumerClient<'_>) {
        let keeper = Address::generate(env);
        let consumer_addr = env.register_contract(None, TwalConsumer);
        let consumer = TwalConsumerClient::new(env, &consumer_addr);
        consumer.initialize(&keeper);
        (keeper, consumer)
    }

    #[test]
    fn test_add_tracked_pool_is_idempotent() {
        let env = Env::default();
        env.mock_all_auths();
        let (_keeper, consumer) = setup_consumer(&env);
        let pool = Address::generate(&env);

        assert!(!consumer.is_tracked(&pool));
        consumer.add_tracked_pool(&pool);
        assert!(consumer.is_tracked(&pool));
        assert_eq!(consumer.get_tracked_pool_count(), 1);

        // Adding again is a no-op, not an error, and count does not grow.
        consumer.add_tracked_pool(&pool);
        assert_eq!(consumer.get_tracked_pool_count(), 1);
    }

    #[test]
    fn test_add_tracked_pool_rejects_non_keeper() {
        let env = Env::default();
        let (_keeper, consumer) = setup_consumer(&env);
        let pool = Address::generate(&env);

        // No auths mocked: the keeper's require_auth() must reject this.
        assert!(consumer.try_add_tracked_pool(&pool).is_err());
    }

    #[test]
    fn test_remove_tracked_pool_rejects_non_keeper() {
        let env = Env::default();
        env.mock_all_auths();
        let (_keeper, consumer) = setup_consumer(&env);
        let pool = Address::generate(&env);
        consumer.add_tracked_pool(&pool);
        env.set_auths(&[]);

        assert!(consumer.try_remove_tracked_pool(&pool).is_err());
    }

    #[test]
    fn test_add_tracked_pool_rejects_beyond_max() {
        let env = Env::default();
        env.mock_all_auths();
        let (_keeper, consumer) = setup_consumer(&env);

        for _ in 0..TwalConsumer::MAX_TRACKED_POOLS {
            consumer.add_tracked_pool(&Address::generate(&env));
        }
        assert_eq!(
            consumer.get_tracked_pool_count(),
            TwalConsumer::MAX_TRACKED_POOLS
        );

        let overflow_pool = Address::generate(&env);
        assert_eq!(
            consumer.try_add_tracked_pool(&overflow_pool),
            Err(Ok(TwalError::TooManyTrackedPools))
        );
        assert!(!consumer.is_tracked(&overflow_pool));
    }

    #[test]
    fn test_remove_tracked_pool_preserves_order_of_rest() {
        let env = Env::default();
        env.mock_all_auths();
        let (_keeper, consumer) = setup_consumer(&env);

        let pools = [
            Address::generate(&env),
            Address::generate(&env),
            Address::generate(&env),
            Address::generate(&env),
            Address::generate(&env),
        ];
        for p in pools.iter() {
            consumer.add_tracked_pool(p);
        }

        // Remove the middle pool; the rest must keep their relative order.
        consumer.remove_tracked_pool(&pools[2]);

        let remaining = consumer.get_tracked_pools_paginated(&0, &10);
        assert_eq!(remaining.len(), 4);
        assert_eq!(remaining.get(0).unwrap(), pools[0]);
        assert_eq!(remaining.get(1).unwrap(), pools[1]);
        assert_eq!(remaining.get(2).unwrap(), pools[3]);
        assert_eq!(remaining.get(3).unwrap(), pools[4]);
        assert!(!consumer.is_tracked(&pools[2]));
    }

    #[test]
    fn test_remove_tracked_pool_errors_when_not_tracked() {
        let env = Env::default();
        env.mock_all_auths();
        let (_keeper, consumer) = setup_consumer(&env);
        let pool = Address::generate(&env);

        assert_eq!(
            consumer.try_remove_tracked_pool(&pool),
            Err(Ok(TwalError::NotTracked))
        );
    }

    #[test]
    fn test_pagination_offset_equals_count_and_limit_zero() {
        let env = Env::default();
        env.mock_all_auths();
        let (_keeper, consumer) = setup_consumer(&env);
        let pool = Address::generate(&env);
        consumer.add_tracked_pool(&pool);

        assert_eq!(consumer.get_tracked_pools_paginated(&1, &10).len(), 0);
        assert_eq!(consumer.get_tracked_pools_paginated(&0, &0).len(), 0);
        assert_eq!(consumer.get_tracked_pools_paginated(&0, &10).len(), 1);
    }

    #[test]
    fn test_pagination_on_empty_set() {
        let env = Env::default();
        let (_keeper, consumer) = setup_consumer(&env);

        assert_eq!(consumer.get_tracked_pools_paginated(&0, &10).len(), 0);
        assert_eq!(consumer.get_tracked_pool_count(), 0);
    }

    #[test]
    fn test_implicit_registration_via_save_snapshot_respects_cap() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(10_000);

        let admin = Address::generate(&env);
        let amm_addr = env.register_contract(None, AmmPool);
        let lp_addr = env.register_contract(None, LpToken);

        token::LpTokenClient::new(&env, &lp_addr).initialize(
            &amm_addr,
            &soroban_sdk::String::from_str(&env, "LP"),
            &soroban_sdk::String::from_str(&env, "LP"),
            &7u32,
        );
        let (ta, ta_sac) = create_sac(&env, &admin);
        let (tb, tb_sac) = create_sac(&env, &admin);
        AmmPoolClient::new(&env, &amm_addr).initialize(
            &admin,
            &ta.address,
            &tb.address,
            &lp_addr,
            &30_i128,
            &admin,
            &0_i128,
        );
        let provider = Address::generate(&env);
        ta_sac.mint(&provider, &1_000_000_i128);
        tb_sac.mint(&provider, &1_000_000_i128);
        AmmPoolClient::new(&env, &amm_addr).add_liquidity(
            &provider,
            &1_000_000_i128,
            &1_000_000_i128,
            &0_i128,
            &u64::MAX,
        );

        let (_keeper, consumer) = setup_consumer(&env);
        for _ in 0..TwalConsumer::MAX_TRACKED_POOLS {
            consumer.add_tracked_pool(&Address::generate(&env));
        }

        // The pool itself is not yet tracked, so implicit registration inside
        // save_snapshot is what hits the cap.
        assert!(!consumer.is_tracked(&amm_addr));
        assert_eq!(
            consumer.try_save_snapshot(&amm_addr),
            Err(Ok(TwalError::TooManyTrackedPools))
        );
    }

    #[test]
    fn test_window_too_large_rejected_single_and_batch() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(10_000_000);
        let (_keeper, consumer) = setup_consumer(&env);
        let pool = Address::generate(&env);

        let too_big = TwalConsumer::MAX_WINDOW_SECONDS + 1;
        assert_eq!(
            consumer.try_get_twal_liquidity(&pool, &too_big),
            Err(Ok(TwalError::WindowTooLarge))
        );

        let entries = consumer.get_twal_all_safe(&too_big);
        assert_eq!(entries.len(), 0); // pool not tracked, but no panic either

        consumer.add_tracked_pool(&pool);
        let entries = consumer.get_twal_all_safe(&too_big);
        assert_eq!(entries.len(), 1);
        let entry = entries.get(0).unwrap();
        assert!(!entry.ok);
        assert_eq!(entry.error_code, TwalError::WindowTooLarge as u32);
    }

    #[test]
    fn test_get_twal_all_safe_isolates_no_snapshot_pool() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(10_000);
        let (_keeper, consumer) = setup_consumer(&env);

        // Register a real AMM pool as tracked, but never snapshot it.
        let admin = Address::generate(&env);
        let amm_addr = env.register_contract(None, AmmPool);
        let lp_addr = env.register_contract(None, LpToken);
        token::LpTokenClient::new(&env, &lp_addr).initialize(
            &amm_addr,
            &soroban_sdk::String::from_str(&env, "LP"),
            &soroban_sdk::String::from_str(&env, "LP"),
            &7u32,
        );
        let (ta, _ta_sac) = create_sac(&env, &admin);
        let (tb, _tb_sac) = create_sac(&env, &admin);
        AmmPoolClient::new(&env, &amm_addr).initialize(
            &admin,
            &ta.address,
            &tb.address,
            &lp_addr,
            &30_i128,
            &admin,
            &0_i128,
        );
        consumer.add_tracked_pool(&amm_addr);

        env.ledger().with_mut(|l| l.timestamp = 10_600);
        let entries = consumer.get_twal_all_safe(&600);
        assert_eq!(entries.len(), 1);
        let entry = entries.get(0).unwrap();
        assert!(!entry.ok);
        assert_eq!(entry.error_code, TwalError::NoSnapshotFound as u32);
        assert_eq!(entry.pool, amm_addr);
    }

    #[test]
    fn test_get_twal_all_safe_isolates_panicking_pool_not_transaction_failure() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(10_000);
        let (_keeper, consumer) = setup_consumer(&env);

        let panic_pool = env.register_contract(None, PanicPool);
        consumer.add_tracked_pool(&panic_pool);

        // Must not panic/abort the call despite the tracked pool's oracle
        // call always panicking.
        let entries = consumer.get_twal_all_safe(&600);
        assert_eq!(entries.len(), 1);
        let entry = entries.get(0).unwrap();
        assert!(!entry.ok);
        assert_eq!(entry.error_code, TwalError::CrossContractCallFailed as u32);
        assert_eq!(entry.twal, 0);
    }

    #[test]
    fn test_get_twal_all_safe_isolates_non_contract_address() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(10_000);
        let (_keeper, consumer) = setup_consumer(&env);

        // An address that was never registered as a contract at all.
        let not_a_contract = Address::generate(&env);
        consumer.add_tracked_pool(&not_a_contract);

        let entries = consumer.get_twal_all_safe(&600);
        assert_eq!(entries.len(), 1);
        let entry = entries.get(0).unwrap();
        assert!(!entry.ok);
        assert_eq!(entry.error_code, TwalError::CrossContractCallFailed as u32);
    }

    #[test]
    fn test_get_twal_all_still_errs_when_any_pool_fails() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(10_000);
        let (_keeper, consumer) = setup_consumer(&env);

        let panic_pool = env.register_contract(None, PanicPool);
        consumer.add_tracked_pool(&panic_pool);

        assert!(consumer.try_get_twal_all(&600).is_err());
    }

    #[test]
    fn test_get_twal_all_ok_matches_get_twal_liquidity_when_all_succeed() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(10_000);

        let admin = Address::generate(&env);
        let amm_addr = env.register_contract(None, AmmPool);
        let lp_addr = env.register_contract(None, LpToken);
        token::LpTokenClient::new(&env, &lp_addr).initialize(
            &amm_addr,
            &soroban_sdk::String::from_str(&env, "LP"),
            &soroban_sdk::String::from_str(&env, "LP"),
            &7u32,
        );
        let (ta, ta_sac) = create_sac(&env, &admin);
        let (tb, tb_sac) = create_sac(&env, &admin);
        AmmPoolClient::new(&env, &amm_addr).initialize(
            &admin,
            &ta.address,
            &tb.address,
            &lp_addr,
            &30_i128,
            &admin,
            &0_i128,
        );
        let provider = Address::generate(&env);
        ta_sac.mint(&provider, &1_000_000_i128);
        tb_sac.mint(&provider, &1_000_000_i128);
        AmmPoolClient::new(&env, &amm_addr).add_liquidity(
            &provider,
            &1_000_000_i128,
            &1_000_000_i128,
            &0_i128,
            &u64::MAX,
        );

        let (_keeper, consumer) = setup_consumer(&env);
        consumer.save_snapshot(&amm_addr);

        env.ledger().with_mut(|l| l.timestamp = 10_600);
        ta_sac.mint(&provider, &200_000_i128);
        tb_sac.mint(&provider, &200_000_i128);
        AmmPoolClient::new(&env, &amm_addr).add_liquidity(
            &provider,
            &100_000_i128,
            &100_000_i128,
            &0_i128,
            &u64::MAX,
        );
        consumer.save_snapshot(&amm_addr);

        env.ledger().with_mut(|l| l.timestamp = 11_200);
        // Advance the pool's internal timestamp past the last snapshot so
        // `elapsed > 0`, matching the pattern used elsewhere in this file.
        let trader = Address::generate(&env);
        ta_sac.mint(&trader, &1_000_i128);
        AmmPoolClient::new(&env, &amm_addr).swap(
            &trader,
            &ta.address,
            &1_000_i128,
            &0_i128,
            &u64::MAX,
        );

        let expected = consumer.get_twal_liquidity(&amm_addr, &600);
        let all = consumer.get_twal_all(&600);
        assert_eq!(all.len(), 1);
        assert_eq!(all.get(0).unwrap(), (amm_addr, expected));
    }

    #[test]
    fn test_get_twal_batch_reads_caller_supplied_subset() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(10_000);
        let (_keeper, consumer) = setup_consumer(&env);

        // Not part of the global tracked set at all.
        let panic_pool = env.register_contract(None, PanicPool);
        let not_a_contract = Address::generate(&env);
        let pools: Vec<Address> =
            Vec::from_array(&env, [panic_pool.clone(), not_a_contract.clone()]);

        assert_eq!(consumer.get_tracked_pool_count(), 0);
        let entries = consumer.get_twal_batch(&pools, &600);
        assert_eq!(entries.len(), 2);
        assert!(!entries.get(0).unwrap().ok);
        assert!(!entries.get(1).unwrap().ok);
    }

    #[test]
    fn test_get_twal_batch_rejects_oversized_input() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(10_000);
        let (_keeper, consumer) = setup_consumer(&env);

        let mut pools: Vec<Address> = Vec::new(&env);
        for _ in 0..(TwalConsumer::MAX_TRACKED_POOLS + 1) {
            pools.push_back(Address::generate(&env));
        }

        assert_eq!(
            consumer.try_get_twal_batch(&pools, &600),
            Err(Ok(TwalError::TooManyPools))
        );
    }
}
