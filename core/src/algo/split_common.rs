use std::collections::HashMap;

use alloy_primitives::{Address, U256};

use crate::graph::Graph;
use crate::pool::Pool;

// Sequential-simulation primitives shared by `split_dp` and `split_fw`.
// Given an allocation (token-paths + input amounts), simulate atomic
// execution against a mutating copy of the pool reserves so that
// shared-pool depletion is correctly reflected.

#[derive(Clone, Debug)]
pub struct LegSim {
    pub amount_in: U256,
    pub amount_out: U256,
    pub pool_idxs: Vec<usize>,
}

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

// Constant-product output with overridden reserves.
pub fn output_with_reserves(
    pool: &Pool,
    in_token: Address,
    amount_in: U256,
    r_in: U256,
    r_out: U256,
) -> U256 {
    if amount_in.is_zero() || r_in.is_zero() || r_out.is_zero() {
        return U256::ZERO;
    }
    let _ = in_token;
    let fee_denom = U256::from(10_000u64);
    let fee_mult = U256::from(10_000u64 - pool.fee_bps as u64);
    let amount_in_with_fee = amount_in * fee_mult;
    let numerator = amount_in_with_fee * r_out;
    let denominator = r_in * fee_denom + amount_in_with_fee;
    if denominator.is_zero() {
        return U256::ZERO;
    }
    numerator / denominator
}

pub type PoolState = HashMap<usize, (U256, U256)>;

pub fn initial_state(graph: &Graph) -> PoolState {
    graph
        .pools
        .iter()
        .enumerate()
        .map(|(i, p)| (i, (p.reserve_a, p.reserve_b)))
        .collect()
}

// Simulate an allocation sequentially, biggest leg first (canonical
// atomic-transaction order — later legs see post-impact reserves).
// Returns total output and per-leg detail; legs skipped on zero input
// or missing pools come back as None.
pub fn simulate_allocation(
    graph: &Graph,
    paths: &[Vec<usize>],
    inputs: &[U256],
    by_pair: &HashMap<(usize, usize), Vec<usize>>,
) -> (U256, Vec<Option<LegSim>>) {
    let mut state = initial_state(graph);
    simulate_allocation_mut(graph, paths, inputs, by_pair, &mut state)
}

// As `simulate_allocation` but with caller-supplied state, so the
// post-flow reserves can be inspected (FW uses this to compute the
// next direction's input graph).
pub fn simulate_allocation_mut(
    graph: &Graph,
    paths: &[Vec<usize>],
    inputs: &[U256],
    by_pair: &HashMap<(usize, usize), Vec<usize>>,
    state: &mut PoolState,
) -> (U256, Vec<Option<LegSim>>) {
    assert_eq!(paths.len(), inputs.len());
    let mut order: Vec<usize> = (0..paths.len()).collect();
    order.sort_by(|&a, &b| inputs[b].cmp(&inputs[a]));

    let mut per_leg: Vec<Option<LegSim>> = vec![None; paths.len()];
    let mut total = U256::ZERO;

    for &leg_i in &order {
        let a_in = inputs[leg_i];
        if a_in.is_zero() {
            continue;
        }
        let path = &paths[leg_i];
        if path.len() < 2 {
            continue;
        }

        let mut amount = a_in;
        let mut pool_idxs: Vec<usize> = Vec::with_capacity(path.len() - 1);
        let mut valid = true;

        for hop in 0..(path.len() - 1) {
            let u = path[hop];
            let v = path[hop + 1];
            let key = if u < v { (u, v) } else { (v, u) };
            let Some(pool_candidates) = by_pair.get(&key) else {
                valid = false;
                break;
            };
            let from_addr = graph.tokens[u].address;

            let mut best_pool: Option<usize> = None;
            let mut best_out = U256::ZERO;
            for &pool_idx in pool_candidates {
                let pool = &graph.pools[pool_idx];
                let (ra, rb) = state[&pool_idx];
                let (r_in, r_out) = if pool.token_a == from_addr {
                    (ra, rb)
                } else {
                    (rb, ra)
                };
                let out = output_with_reserves(pool, from_addr, amount, r_in, r_out);
                if best_pool.is_none() || out > best_out {
                    best_pool = Some(pool_idx);
                    best_out = out;
                }
            }
            let Some(pool_idx) = best_pool else {
                valid = false;
                break;
            };

            let pool = &graph.pools[pool_idx];
            let (ra, rb) = state[&pool_idx];
            let new_state = if pool.token_a == from_addr {
                (ra + amount, rb.saturating_sub(best_out))
            } else {
                (ra.saturating_sub(best_out), rb + amount)
            };
            state.insert(pool_idx, new_state);

            pool_idxs.push(pool_idx);
            amount = best_out;
        }

        if valid && !amount.is_zero() {
            per_leg[leg_i] = Some(LegSim {
                amount_in: a_in,
                amount_out: amount,
                pool_idxs,
            });
            total += amount;
        }
    }

    (total, per_leg)
}

// `amount * frac` at 6-decimal-digit precision in U256.
pub fn fractional_mul(amount: U256, frac: f64) -> U256 {
    if frac <= 0.0 {
        return U256::ZERO;
    }
    if frac >= 1.0 {
        return amount;
    }
    let scale = 1_000_000u64;
    let numer = (frac * scale as f64).round() as u64;
    amount * U256::from(numer) / U256::from(scale)
}
