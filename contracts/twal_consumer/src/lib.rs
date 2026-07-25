#![no_std]

//! TWAL (time-weighted average liquidity) consumer contract.
//!
//! Mirrors the `twap_consumer` pattern: keepers save periodic snapshots of each
//! pool's on-chain `get_liquidity_cumulative` accumulator, then callers query
//! average liquidity over a window for yield calculations and multi-pool analytics.

use soroban_sdk::{contract, contractclient, contractimpl, contracttype, Address, Env, Vec};

#[contractclient(name = "AmmPoolLiquidityClient")]
pub trait AmmPoolLiquidityOracle {
    fn get_liquidity_cumulative(env: Env) -> (i128, u64);
}

#[contractclient(name = "ClPoolLiquidityClient")]
pub trait ClPoolLiquidityOracle {
    fn active_liquidity(env: Env) -> i128;
    fn get_tick_cumulative(env: Env) -> (i64, u64);
}

#[contracttype]
pub enum DataKey {
    LiquiditySnapshot(Address, u64),
    TrackedPools,
    /// Running liquidity-cumulative state for a CL pool (see `save_cl_snapshot`).
    ClAccumulator(Address),
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LiquiditySnapshot {
    pub cum_liquidity: i128,
    pub pool_ts: u64,
}

/// Running integral of a CL pool's active liquidity over ledger time.
///
/// Concentrated-liquidity pools only expose the *instantaneous* active
/// liquidity, so — unlike the AMM path — there is no pool-side cumulative to
/// difference. This contract builds one itself: on each keeper snapshot it adds
/// `last_active * elapsed` to `cum_liquidity`, turning a series of instantaneous
/// readings into a proper time-weighted accumulator that `get_cl_twal` can
/// difference to recover an average liquidity *level*.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClLiquidityAccumulator {
    /// Integral of active liquidity over ledger time (liquidity-seconds).
    pub cum_liquidity: i128,
    /// Ledger timestamp at which the accumulator was last updated.
    pub last_ts: u64,
    /// Active liquidity recorded at `last_ts`, held constant until the next update.
    pub last_active: i128,
}

#[contract]
pub struct TwalConsumer;

#[contractimpl]
impl TwalConsumer {
    pub const SNAPSHOT_TTL_LEDGERS: u32 = 120_960;

    /// Persist a pool liquidity accumulator snapshot keyed by ledger timestamp.
    pub fn save_snapshot(env: Env, pool: Address) {
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

        let mut tracked: Vec<Address> = env
            .storage()
            .instance()
            .get(&DataKey::TrackedPools)
            .unwrap_or_else(|| Vec::new(&env));
        let mut already = false;
        for i in 0..tracked.len() {
            if tracked.get(i).unwrap() == pool {
                already = true;
                break;
            }
        }
        if !already {
            tracked.push_back(pool);
            env.storage()
                .instance()
                .set(&DataKey::TrackedPools, &tracked);
        }
    }

    /// Average pool liquidity (sqrt(reserve_a * reserve_b)) over `window_seconds`.
    pub fn get_twal_liquidity(env: Env, pool: Address, window_seconds: u64) -> i128 {
        assert!(window_seconds > 0, "window_seconds must be > 0");

        let (cum_now, pool_ts_now) =
            AmmPoolLiquidityClient::new(&env, &pool).get_liquidity_cumulative();
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

        let delta = (cum_now as u128).wrapping_sub(snapshot.cum_liquidity as u128) as i128;
        let elapsed = (pool_ts_now - snapshot.pool_ts) as i128;
        assert!(elapsed > 0, "window too small (pool time did not advance)");
        delta / elapsed
    }

    /// TWAL for every tracked pool in one call.
    pub fn get_twal_all(env: Env, window_seconds: u64) -> Vec<(Address, i128)> {
        let tracked = Self::get_tracked_pools(env.clone());
        let mut results: Vec<(Address, i128)> = Vec::new(&env);
        for i in 0..tracked.len() {
            let pool = tracked.get(i).unwrap();
            let twal = Self::get_twal_liquidity(env.clone(), pool.clone(), window_seconds);
            results.push_back((pool, twal));
        }
        results
    }

    pub fn get_tracked_pools(env: Env) -> Vec<Address> {
        env.storage()
            .instance()
            .get(&DataKey::TrackedPools)
            .unwrap_or_else(|| Vec::new(&env))
    }

    /// Save a CL pool snapshot, accumulating the integral of active liquidity.
    ///
    /// A CL pool only exposes its *instantaneous* active liquidity, so each call
    /// advances a running accumulator by `last_active * elapsed` (the liquidity
    /// recorded at the previous snapshot, held constant over the interval since).
    /// The resulting cumulative — not the raw instantaneous reading — is stored
    /// in the snapshot, so `get_cl_twal` can difference two snapshots to recover
    /// an average liquidity *level* rather than a rate of change.
    pub fn save_cl_snapshot(env: Env, pool: Address) {
        let active = ClPoolLiquidityClient::new(&env, &pool).active_liquidity();
        let ledger_ts = env.ledger().timestamp();

        let acc_key = DataKey::ClAccumulator(pool.clone());
        let cum = match env
            .storage()
            .persistent()
            .get::<_, ClLiquidityAccumulator>(&acc_key)
        {
            Some(prev) => {
                let elapsed = ledger_ts.saturating_sub(prev.last_ts) as i128;
                prev.cum_liquidity + prev.last_active * elapsed
            }
            // First snapshot for this pool: cumulative starts at zero.
            None => 0,
        };

        let accumulator = ClLiquidityAccumulator {
            cum_liquidity: cum,
            last_ts: ledger_ts,
            last_active: active,
        };
        env.storage().persistent().set(&acc_key, &accumulator);
        env.storage().persistent().extend_ttl(
            &acc_key,
            Self::SNAPSHOT_TTL_LEDGERS / 2,
            Self::SNAPSHOT_TTL_LEDGERS,
        );

        let snapshot = LiquiditySnapshot {
            cum_liquidity: cum,
            pool_ts: ledger_ts,
        };
        let key = DataKey::LiquiditySnapshot(pool.clone(), ledger_ts);
        env.storage().persistent().set(&key, &snapshot);
        env.storage().persistent().extend_ttl(
            &key,
            Self::SNAPSHOT_TTL_LEDGERS / 2,
            Self::SNAPSHOT_TTL_LEDGERS,
        );

        let mut tracked: Vec<Address> = env
            .storage()
            .instance()
            .get(&DataKey::TrackedPools)
            .unwrap_or_else(|| Vec::new(&env));
        let mut already = false;
        for i in 0..tracked.len() {
            if tracked.get(i).unwrap() == pool {
                already = true;
                break;
            }
        }
        if !already {
            tracked.push_back(pool);
            env.storage()
                .instance()
                .set(&DataKey::TrackedPools, &tracked);
        }
    }

    /// Average active liquidity of a CL pool over `window_seconds`.
    ///
    /// Differences the liquidity-cumulative saved `window_seconds` ago against
    /// the accumulator extrapolated to the current ledger time, then divides by
    /// the window. A pool whose active liquidity is constant at `L` correctly
    /// reports `L` (the earlier code reported the slope, i.e. 0 for constant
    /// liquidity). Requires a snapshot saved at approximately `now - window` and
    /// at least one prior `save_cl_snapshot` establishing the accumulator.
    pub fn get_cl_twal(env: Env, pool: Address, window_seconds: u64) -> i128 {
        assert!(window_seconds > 0, "window_seconds must be > 0");

        let ledger_ts_now = env.ledger().timestamp();
        assert!(
            ledger_ts_now >= window_seconds,
            "ledger timestamp is smaller than requested window"
        );

        let then_ts = ledger_ts_now - window_seconds;
        let snapshot: LiquiditySnapshot = env
            .storage()
            .persistent()
            .get(&DataKey::LiquiditySnapshot(pool.clone(), then_ts))
            .unwrap_or_else(|| panic!("missing liquidity snapshot at {then_ts}"));

        let accumulator: ClLiquidityAccumulator = env
            .storage()
            .persistent()
            .get(&DataKey::ClAccumulator(pool))
            .unwrap_or_else(|| panic!("missing CL accumulator; call save_cl_snapshot first"));

        // Extrapolate the cumulative to now using the last recorded active
        // liquidity, mirroring how the AMM accumulator advances between checkpoints.
        let elapsed_since_update = ledger_ts_now.saturating_sub(accumulator.last_ts) as i128;
        let cum_now = accumulator.cum_liquidity + accumulator.last_active * elapsed_since_update;

        let elapsed = (ledger_ts_now - then_ts) as i128;
        assert!(elapsed > 0, "window too small");
        (cum_now - snapshot.cum_liquidity) / elapsed
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
        AmmPoolClient::new(&env, &amm_addr)
            .initialize(
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
        consumer.save_snapshot(&amm_addr);

        env.ledger().with_mut(|l| l.timestamp = 10_600);
        // Mint additional tokens for the second deposit.
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
        // Trigger a pool interaction so checkpoint_twal advances pool_ts to 11_200.
        let trader = Address::generate(&env);
        ta_sac.mint(&trader, &1_000_i128);
        AmmPoolClient::new(&env, &amm_addr).swap(&trader, &ta.address, &1_000_i128, &0_i128, &u64::MAX);

        let twal = consumer.get_twal_liquidity(&amm_addr, &600);
        assert!(twal > 0);
    }

    // Minimal CL pool stand-in exposing a settable instantaneous active
    // liquidity, matching the `ClPoolLiquidityOracle::active_liquidity` method
    // the consumer reads.
    #[contract]
    pub struct MockClPool;

    #[contractimpl]
    impl MockClPool {
        pub fn set_liquidity(env: Env, value: i128) {
            env.storage()
                .instance()
                .set(&soroban_sdk::symbol_short!("liq"), &value);
        }

        pub fn active_liquidity(env: Env) -> i128 {
            env.storage()
                .instance()
                .get(&soroban_sdk::symbol_short!("liq"))
                .unwrap_or(0)
        }
    }

    #[test]
    fn test_cl_twal_constant_liquidity_reports_level() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(10_000);

        let pool_addr = env.register_contract(None, MockClPool);
        let pool = MockClPoolClient::new(&env, &pool_addr);
        let consumer_addr = env.register_contract(None, TwalConsumer);
        let consumer = TwalConsumerClient::new(&env, &consumer_addr);

        // Active liquidity is constant at 5_000 across the whole window.
        pool.set_liquidity(&5_000_i128);
        consumer.save_cl_snapshot(&pool_addr); // baseline accumulator at t=10_000

        env.ledger().with_mut(|l| l.timestamp = 10_600);
        consumer.save_cl_snapshot(&pool_addr); // accumulator advances by 5_000*600

        env.ledger().with_mut(|l| l.timestamp = 11_200);
        let twal = consumer.get_cl_twal(&pool_addr, &600);

        // Constant liquidity must report its level, not a rate of change (0).
        assert_eq!(twal, 5_000);
    }

    #[test]
    fn test_cl_twal_is_time_weighted_average_of_levels() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(10_000);

        let pool_addr = env.register_contract(None, MockClPool);
        let pool = MockClPoolClient::new(&env, &pool_addr);
        let consumer_addr = env.register_contract(None, TwalConsumer);
        let consumer = TwalConsumerClient::new(&env, &consumer_addr);

        // 1_000 for the first 300s, then 3_000 for the next 300s.
        pool.set_liquidity(&1_000_i128);
        consumer.save_cl_snapshot(&pool_addr);

        env.ledger().with_mut(|l| l.timestamp = 10_300);
        pool.set_liquidity(&3_000_i128);
        consumer.save_cl_snapshot(&pool_addr);

        env.ledger().with_mut(|l| l.timestamp = 10_600);
        consumer.save_cl_snapshot(&pool_addr);

        // Window covers both segments: (1_000*300 + 3_000*300) / 600 = 2_000.
        let twal = consumer.get_cl_twal(&pool_addr, &600);
        assert_eq!(twal, 2_000);
    }
}
