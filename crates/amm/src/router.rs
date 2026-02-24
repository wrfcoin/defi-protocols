use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use wrfcoin_defi_common::error::{DefiError, DefiResult};
use wrfcoin_defi_common::types::{Amount, AssetId, PoolId};

use crate::pool::LiquidityPool;
use crate::swap::SwapEngine;

/// A single hop in a multi-hop swap route.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteHop {
    /// The pool to swap through.
    pub pool_id: PoolId,
    /// Whether this hop swaps asset_a -> asset_b (true) or asset_b -> asset_a (false).
    pub is_a_to_b: bool,
}

/// Router for discovering and executing multi-hop swap routes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Router {
    /// Registry of all known liquidity pools, keyed by pool ID.
    pub pools: HashMap<PoolId, LiquidityPool>,
    /// Index: asset -> list of pool IDs that contain this asset.
    asset_to_pools: HashMap<AssetId, Vec<PoolId>>,
}

impl Router {
    pub fn new() -> Self {
        Self {
            pools: HashMap::new(),
            asset_to_pools: HashMap::new(),
        }
    }

    /// Register a pool with the router.
    pub fn add_pool(&mut self, pool: LiquidityPool) {
        let pool_id = pool.pool_id;
        self.asset_to_pools
            .entry(pool.asset_a.clone())
            .or_default()
            .push(pool_id);
        self.asset_to_pools
            .entry(pool.asset_b.clone())
            .or_default()
            .push(pool_id);
        self.pools.insert(pool_id, pool);
    }

    /// Find a route from `asset_in` to `asset_out` with at most `max_hops` hops.
    ///
    /// Uses a simple BFS to find the shortest path. Returns None if no route exists.
    pub fn find_route(
        &self,
        asset_in: &AssetId,
        asset_out: &AssetId,
        max_hops: usize,
    ) -> Option<Vec<RouteHop>> {
        if asset_in == asset_out {
            return Some(vec![]);
        }

        // BFS: queue entries are (current_asset, path_so_far).
        let mut queue: Vec<(AssetId, Vec<RouteHop>)> = vec![(asset_in.clone(), vec![])];
        let mut visited: Vec<AssetId> = vec![asset_in.clone()];

        while let Some((current_asset, path)) = queue.first().cloned() {
            queue.remove(0);

            if path.len() >= max_hops {
                continue;
            }

            let pool_ids = match self.asset_to_pools.get(&current_asset) {
                Some(ids) => ids.clone(),
                None => continue,
            };

            for pool_id in pool_ids {
                let pool = match self.pools.get(&pool_id) {
                    Some(p) => p,
                    None => continue,
                };

                // Determine the other asset in this pool.
                let (next_asset, is_a_to_b) = if pool.asset_a == current_asset {
                    (pool.asset_b.clone(), true)
                } else {
                    (pool.asset_a.clone(), false)
                };

                if visited.contains(&next_asset) {
                    continue;
                }

                let mut new_path = path.clone();
                new_path.push(RouteHop {
                    pool_id,
                    is_a_to_b,
                });

                if next_asset == *asset_out {
                    return Some(new_path);
                }

                visited.push(next_asset.clone());
                queue.push((next_asset, new_path));
            }
        }

        None
    }

    /// Execute a multi-hop swap along a pre-computed route.
    ///
    /// Returns the final output amount. Fails if the final output is below `min_amount_out`.
    pub fn execute_multi_hop(
        &mut self,
        route: &[RouteHop],
        amount_in: Amount,
        min_amount_out: Amount,
    ) -> DefiResult<Amount> {
        if route.is_empty() {
            return Err(DefiError::InvalidAmount {
                reason: "empty route".into(),
            });
        }

        let engine = SwapEngine::new();
        let mut current_amount = amount_in;

        for (i, hop) in route.iter().enumerate() {
            let pool = self.pools.get_mut(&hop.pool_id).ok_or(DefiError::InvalidPool {
                pool_id: hop.pool_id,
            })?;

            // For intermediate hops, no slippage check (set min to 0).
            // Only enforce slippage on the final hop.
            let min_out = if i == route.len() - 1 {
                min_amount_out
            } else {
                Amount::ZERO
            };

            current_amount = engine.execute_swap(pool, current_amount, hop.is_a_to_b, min_out)?;
        }

        Ok(current_amount)
    }

    /// Simulate a multi-hop swap to get the expected output without modifying state.
    pub fn get_expected_output(
        &self,
        route: &[RouteHop],
        amount_in: Amount,
    ) -> DefiResult<Amount> {
        let engine = SwapEngine::new();
        let mut current_amount = amount_in;

        for hop in route {
            let pool = self.pools.get(&hop.pool_id).ok_or(DefiError::InvalidPool {
                pool_id: hop.pool_id,
            })?;

            let (reserve_in, reserve_out) = if hop.is_a_to_b {
                (pool.reserve_a, pool.reserve_b)
            } else {
                (pool.reserve_b, pool.reserve_a)
            };

            current_amount =
                engine.get_amount_out(current_amount, reserve_in, reserve_out, pool.fee_rate_bps)?;
        }

        Ok(current_amount)
    }
}

impl Default for Router {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_pool_with_liquidity(id: u64, a: &str, b: &str, reserve: u128) -> LiquidityPool {
        let mut pool = LiquidityPool::new(PoolId::new(id), AssetId::new(a), AssetId::new(b), 30);
        pool.reserve_a = Amount::new(reserve);
        pool.reserve_b = Amount::new(reserve);
        pool.total_lp_supply = Amount::new(reserve);
        pool
    }

    #[test]
    fn test_find_direct_route() {
        let mut router = Router::new();
        router.add_pool(make_pool_with_liquidity(1, "A", "B", 10_000));

        let route = router.find_route(&AssetId::new("A"), &AssetId::new("B"), 3).unwrap();
        assert_eq!(route.len(), 1);
        assert_eq!(route[0].pool_id, PoolId::new(1));
        assert!(route[0].is_a_to_b);
    }

    #[test]
    fn test_find_reverse_route() {
        let mut router = Router::new();
        router.add_pool(make_pool_with_liquidity(1, "A", "B", 10_000));

        let route = router.find_route(&AssetId::new("B"), &AssetId::new("A"), 3).unwrap();
        assert_eq!(route.len(), 1);
        assert!(!route[0].is_a_to_b);
    }

    #[test]
    fn test_find_multi_hop_route() {
        let mut router = Router::new();
        router.add_pool(make_pool_with_liquidity(1, "A", "B", 10_000));
        router.add_pool(make_pool_with_liquidity(2, "B", "C", 10_000));

        let route = router.find_route(&AssetId::new("A"), &AssetId::new("C"), 3).unwrap();
        assert_eq!(route.len(), 2);
    }

    #[test]
    fn test_no_route() {
        let mut router = Router::new();
        router.add_pool(make_pool_with_liquidity(1, "A", "B", 10_000));

        let route = router.find_route(&AssetId::new("A"), &AssetId::new("Z"), 3);
        assert!(route.is_none());
    }

    #[test]
    fn test_same_asset_route() {
        let router = Router::new();
        let route = router.find_route(&AssetId::new("A"), &AssetId::new("A"), 3).unwrap();
        assert!(route.is_empty());
    }

    #[test]
    fn test_execute_multi_hop() {
        let mut router = Router::new();
        router.add_pool(make_pool_with_liquidity(1, "A", "B", 10_000));
        router.add_pool(make_pool_with_liquidity(2, "B", "C", 10_000));

        let route = router.find_route(&AssetId::new("A"), &AssetId::new("C"), 3).unwrap();
        let out = router.execute_multi_hop(&route, Amount::new(100), Amount::ZERO).unwrap();
        assert!(out.0 > 0);
    }

    #[test]
    fn test_execute_empty_route_fails() {
        let mut router = Router::new();
        let result = router.execute_multi_hop(&[], Amount::new(100), Amount::ZERO);
        assert!(result.is_err());
    }

    #[test]
    fn test_get_expected_output() {
        let mut router = Router::new();
        router.add_pool(make_pool_with_liquidity(1, "A", "B", 10_000));

        let route = router.find_route(&AssetId::new("A"), &AssetId::new("B"), 1).unwrap();
        let expected = router.get_expected_output(&route, Amount::new(100)).unwrap();
        assert!(expected.0 > 0);
    }
}
