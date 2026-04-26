use std::collections::HashMap;

use alloy_primitives::U256;

use crate::graph::Graph;

// Reconstruct (token_path, pool_path) from a Dijkstra/BF prev[] table.
// Panics if dst is unreachable — caller must gate on `prev[dst].is_some()`.
pub fn reconstruct_with_pools(
    prev: &[Option<(usize, usize)>],
    src: usize,
    dst: usize,
) -> (Vec<usize>, Vec<usize>) {
    if src == dst {
        return (vec![src], Vec::new());
    }
    let mut path = vec![dst];
    let mut pools = Vec::new();
    let mut current = dst;
    while current != src {
        let (parent, pool_idx) = prev[current]
            .expect("reconstruct_with_pools: dst unreachable — caller must gate on prev[dst]");
        pools.push(pool_idx);
        path.push(parent);
        current = parent;
    }
    path.reverse();
    pools.reverse();
    (path, pools)
}

// (token_a, token_b) → pool indices. Duplicates `split_common::build_by_pair`
// so single-path algorithms stay independent of the split-router module.
pub fn build_by_pair(graph: &Graph) -> HashMap<(usize, usize), Vec<usize>> {
    let mut by_pair: HashMap<(usize, usize), Vec<usize>> =
        HashMap::with_capacity(graph.pools.len());
    for (i, pool) in graph.pools.iter().enumerate() {
        let Some(a) = graph.index_of(pool.token_a) else {
            continue;
        };
        let Some(b) = graph.index_of(pool.token_b) else {
            continue;
        };
        let key = if a < b { (a, b) } else { (b, a) };
        by_pair.entry(key).or_default().push(i);
    }
    by_pair
}

// Walk an explicit (token-path, pool-path) using the given pool at
// each hop — no per-hop pool selection. Returns (final_amount,
// log_weight_sum). None if any pool is unrelated to its hop's tokens
// or if intermediate output rounds to zero.
pub fn walk_pool_path(
    graph: &Graph,
    token_path: &[usize],
    pool_path: &[usize],
    amount_in: U256,
) -> Option<(U256, f64)> {
    if token_path.len() < 2 || pool_path.len() != token_path.len() - 1 {
        return None;
    }
    let mut amount = amount_in;
    let mut log_weight = 0.0f64;
    for hop in 0..pool_path.len() {
        let from_addr = graph.tokens[token_path[hop]].address;
        let pool = &graph.pools[pool_path[hop]];
        let to_addr = graph.tokens[token_path[hop + 1]].address;
        // Sanity: the pool must actually connect (from, to).
        let valid = (pool.token_a == from_addr && pool.token_b == to_addr)
            || (pool.token_a == to_addr && pool.token_b == from_addr);
        if !valid {
            return None;
        }
        amount = pool.output_amount(from_addr, amount);
        if amount.is_zero() {
            return None;
        }
        log_weight += pool.log_weight(from_addr);
    }
    Some((amount, log_weight))
}

// Walk a token-path picking the best parallel pool at each hop given
// the running amount. Returns (pool_idxs, final_amount, log_weight_sum).
// This is the slippage-aware pool selection every algorithm uses to
// turn a token-path + trade size into a concrete pool-path + output.
pub fn walk_with_best_pools(
    graph: &Graph,
    by_pair: &HashMap<(usize, usize), Vec<usize>>,
    token_path: &[usize],
    amount_in: U256,
) -> Option<(Vec<usize>, U256, f64)> {
    if token_path.len() < 2 {
        return None;
    }
    let mut amount = amount_in;
    let mut pools_idx: Vec<usize> = Vec::with_capacity(token_path.len() - 1);
    let mut log_weight = 0.0f64;
    for hop in 0..(token_path.len() - 1) {
        let u = token_path[hop];
        let v = token_path[hop + 1];
        let key = if u < v { (u, v) } else { (v, u) };
        let candidates = by_pair.get(&key)?;
        let from_addr = graph.tokens[u].address;
        let mut best_pool_idx: Option<usize> = None;
        let mut best_hop_out = U256::ZERO;
        for &pool_idx in candidates {
            let out = graph.pools[pool_idx].output_amount(from_addr, amount);
            if best_pool_idx.is_none() || out > best_hop_out {
                best_pool_idx = Some(pool_idx);
                best_hop_out = out;
            }
        }
        let pool_idx = best_pool_idx?;
        amount = best_hop_out;
        pools_idx.push(pool_idx);
        log_weight += graph.pools[pool_idx].log_weight(from_addr);
    }
    Some((pools_idx, amount, log_weight))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconstruct_linear_path() {
        // src=0 → 1 → 2 → dst=3 via pools 10, 11, 12
        let prev = vec![None, Some((0, 10)), Some((1, 11)), Some((2, 12))];
        let (path, pools) = reconstruct_with_pools(&prev, 0, 3);
        assert_eq!(path, vec![0, 1, 2, 3]);
        assert_eq!(pools, vec![10, 11, 12]);
    }

    #[test]
    fn reconstruct_src_equals_dst() {
        let prev = vec![None, None, None];
        let (path, pools) = reconstruct_with_pools(&prev, 1, 1);
        assert_eq!(path, vec![1]);
        assert!(pools.is_empty());
    }

    #[test]
    fn reconstruct_single_hop() {
        let prev = vec![None, Some((0, 99))];
        let (path, pools) = reconstruct_with_pools(&prev, 0, 1);
        assert_eq!(path, vec![0, 1]);
        assert_eq!(pools, vec![99]);
    }
}
