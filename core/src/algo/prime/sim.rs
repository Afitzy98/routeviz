use alloy_primitives::U256;

use crate::algo::split_common::{
    LegSim, PoolState, fractional_mul, initial_state, output_with_reserves,
};
use crate::graph::Graph;

use super::path::MultiEdgePath;

// Marginal price of a single-pool-per-hop path at zero flow. Used when
// ranking shortcuts and not sensitive to actual trade size.
pub fn marginal_price_at_zero(graph: &Graph, token_path: &[usize], pool_path: &[usize]) -> f64 {
    token_path
        .windows(2)
        .zip(pool_path.iter())
        .fold(1.0f64, |acc, (pair, &pool_idx)| {
            let in_addr = graph.tokens[pair[0]].address;
            acc * graph.pools[pool_idx].marginal_rate(in_addr)
        })
}

// Marginal price of a multi-edge path at the given input flow on the
// pools' BASE reserves. For a hop with parallel pools and weights w_e:
//   f_E(x)  = Σ_e f_e(w_e · x)
//   f'_E(x) = Σ_e w_e · f'_e(w_e · x)
// Chain across hops via the product rule on the simulated amount.
//
// Uses base reserves (via `Pool::marginal_rate_at_flow`) per the paper.
// V1 used post-flow reserves, which double-counts the flow contribution
// to r_in and yields f'(2x) instead of f'(x).
pub(super) fn marginal_price_path(graph: &Graph, path: &MultiEdgePath, flow: U256) -> f64 {
    let mut chain = 1.0f64;
    let mut amount = flow;
    for (hop, hop_pools) in path.pools_at_hop.iter().enumerate() {
        let in_addr = graph.tokens[path.tokens[hop]].address;
        let weights = &path.edge_weights[hop];

        let mut g_hop = 0.0;
        let mut total_out = U256::ZERO;
        for (i, &pool_idx) in hop_pools.iter().enumerate() {
            let w = weights[i];
            if w <= 0.0 {
                continue;
            }
            let part_in = fractional_mul(amount, w);
            let pool = &graph.pools[pool_idx];
            g_hop += w * pool.marginal_rate_at_flow(in_addr, part_in);
            if !part_in.is_zero() {
                total_out += pool.output_amount(in_addr, part_in);
            }
        }

        if !g_hop.is_finite() {
            return 0.0;
        }
        chain *= g_hop;
        amount = total_out;
    }
    chain
}

// Walk a single MultiEdgePath through the given mutable pool state. At
// each hop, fan out by edge_weights to each pool, sum the outputs, then
// pass to the next hop. Returns final amount; updates pool state in place.
pub(super) fn simulate_multi_edge_path_mut(
    graph: &Graph,
    state: &mut PoolState,
    path: &MultiEdgePath,
    amount_in: U256,
) -> U256 {
    let mut amount = amount_in;
    for (hop, pools) in path.pools_at_hop.iter().enumerate() {
        if amount.is_zero() {
            return U256::ZERO;
        }
        let from = graph.tokens[path.tokens[hop]].address;
        let weights = &path.edge_weights[hop];
        let mut total_out = U256::ZERO;
        for (i, &pool_idx) in pools.iter().enumerate() {
            let w = weights[i];
            if w <= 0.0 {
                continue;
            }
            let part_in = fractional_mul(amount, w);
            if part_in.is_zero() {
                continue;
            }
            let pool = &graph.pools[pool_idx];
            let (ra, rb) = state[&pool_idx];
            let (r_in, r_out) = if pool.token_a == from {
                (ra, rb)
            } else {
                (rb, ra)
            };
            let out = output_with_reserves(pool, from, part_in, r_in, r_out);
            if out.is_zero() {
                continue;
            }
            let new_state = if pool.token_a == from {
                (ra + part_in, rb.saturating_sub(out))
            } else {
                (ra.saturating_sub(out), rb + part_in)
            };
            state.insert(pool_idx, new_state);
            total_out += out;
        }
        amount = total_out;
    }
    amount
}

// Sequentially simulate multiple multi-edge paths, biggest-allocation first.
// Matches the canonical atomic-transaction order so later legs see post-
// impact reserves.
pub(super) fn simulate_multi_edge_paths(
    graph: &Graph,
    paths: &[MultiEdgePath],
    amount_in: U256,
) -> (U256, Vec<Option<LegSim>>) {
    let inputs: Vec<U256> = paths
        .iter()
        .map(|p| fractional_mul(amount_in, p.alloc))
        .collect();
    let mut order: Vec<usize> = (0..paths.len()).collect();
    order.sort_by(|&a, &b| inputs[b].cmp(&inputs[a]));

    let mut state = initial_state(graph);
    let mut per_leg: Vec<Option<LegSim>> = vec![None; paths.len()];
    let mut total = U256::ZERO;
    for &i in &order {
        let amt_in = inputs[i];
        if amt_in.is_zero() {
            continue;
        }
        let out = simulate_multi_edge_path_mut(graph, &mut state, &paths[i], amt_in);
        if !out.is_zero() {
            per_leg[i] = Some(LegSim {
                amount_in: amt_in,
                amount_out: out,
                pool_idxs: paths[i].collect_pools(),
            });
            total += out;
        }
    }
    (total, per_leg)
}

pub(super) fn evaluate_gross(graph: &Graph, paths: &[MultiEdgePath], amount_in: U256) -> U256 {
    simulate_multi_edge_paths(graph, paths, amount_in).0
}

// Walk a single multi-edge path on a fresh pool state. Used by ASGM's
// inner-step for evaluating candidate edge_weights without disturbing
// other paths' contributions.
pub(super) fn simulate_path_isolated(graph: &Graph, path: &MultiEdgePath, amount_in: U256) -> U256 {
    let mut state = initial_state(graph);
    simulate_multi_edge_path_mut(graph, &mut state, path, amount_in)
}

// Amount entering hop `hop` of a path given total input. Walks hops 0..hop
// on a fresh state copy so cross-path interference doesn't leak in.
pub(super) fn amount_entering_hop(
    graph: &Graph,
    path: &MultiEdgePath,
    hop: usize,
    amount_in: U256,
) -> U256 {
    if hop == 0 {
        return amount_in;
    }
    let mut amount = amount_in;
    let mut state = initial_state(graph);
    for h in 0..hop {
        if amount.is_zero() {
            return U256::ZERO;
        }
        let from = graph.tokens[path.tokens[h]].address;
        let weights = &path.edge_weights[h];
        let mut total_out = U256::ZERO;
        for (i, &pool_idx) in path.pools_at_hop[h].iter().enumerate() {
            let w = weights[i];
            if w <= 0.0 {
                continue;
            }
            let part_in = fractional_mul(amount, w);
            if part_in.is_zero() {
                continue;
            }
            let pool = &graph.pools[pool_idx];
            let (ra, rb) = state[&pool_idx];
            let (r_in, r_out) = if pool.token_a == from {
                (ra, rb)
            } else {
                (rb, ra)
            };
            let out = output_with_reserves(pool, from, part_in, r_in, r_out);
            if out.is_zero() {
                continue;
            }
            let new_state = if pool.token_a == from {
                (ra + part_in, rb.saturating_sub(out))
            } else {
                (ra.saturating_sub(out), rb + part_in)
            };
            state.insert(pool_idx, new_state);
            total_out += out;
        }
        amount = total_out;
    }
    amount
}
