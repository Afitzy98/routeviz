use std::collections::HashMap;

use alloy_primitives::{Address, U256};

use crate::graph::Graph;
use crate::pool::Pool;

// Sequential-simulation primitives shared by `split_dp` and `split_fw`.
// Each leg comes in as an explicit (token-path, pool-path) pair — we
// honour the pool sequence rather than re-picking per hop, so two legs
// that go through different parallel pools on the same pair correctly
// drain different pools.

#[derive(Clone, Debug)]
pub struct LegSim {
    pub amount_in: U256,
    pub amount_out: U256,
    pub pool_idxs: Vec<usize>,
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
// Each leg is a (token_path, pool_path) tuple — pool_path[i] is used
// for hop i with no per-hop pool re-pick.
pub fn simulate_allocation(
    graph: &Graph,
    token_paths: &[Vec<usize>],
    pool_paths: &[Vec<usize>],
    inputs: &[U256],
) -> (U256, Vec<Option<LegSim>>) {
    let mut state = initial_state(graph);
    simulate_allocation_mut(graph, token_paths, pool_paths, inputs, &mut state)
}

// As `simulate_allocation` but with caller-supplied state, so the
// post-flow reserves can be inspected (FW uses this to compute the
// next direction's input graph).
pub fn simulate_allocation_mut(
    graph: &Graph,
    token_paths: &[Vec<usize>],
    pool_paths: &[Vec<usize>],
    inputs: &[U256],
    state: &mut PoolState,
) -> (U256, Vec<Option<LegSim>>) {
    assert_eq!(token_paths.len(), inputs.len());
    assert_eq!(token_paths.len(), pool_paths.len());
    let mut order: Vec<usize> = (0..token_paths.len()).collect();
    order.sort_by(|&a, &b| inputs[b].cmp(&inputs[a]));

    let mut per_leg: Vec<Option<LegSim>> = vec![None; token_paths.len()];
    let mut total = U256::ZERO;

    for &leg_i in &order {
        let a_in = inputs[leg_i];
        if a_in.is_zero() {
            continue;
        }
        let tokens = &token_paths[leg_i];
        let pools = &pool_paths[leg_i];
        if tokens.len() < 2 || pools.len() != tokens.len() - 1 {
            continue;
        }

        let mut amount = a_in;
        let mut valid = true;

        for hop in 0..pools.len() {
            let pool_idx = pools[hop];
            let pool = &graph.pools[pool_idx];
            let from_addr = graph.tokens[tokens[hop]].address;
            let (ra, rb) = state[&pool_idx];
            let (r_in, r_out) = if pool.token_a == from_addr {
                (ra, rb)
            } else {
                (rb, ra)
            };
            let out = output_with_reserves(pool, from_addr, amount, r_in, r_out);
            if out.is_zero() {
                valid = false;
                break;
            }

            let new_state = if pool.token_a == from_addr {
                (ra + amount, rb.saturating_sub(out))
            } else {
                (ra.saturating_sub(out), rb + amount)
            };
            state.insert(pool_idx, new_state);
            amount = out;
        }

        if valid && !amount.is_zero() {
            per_leg[leg_i] = Some(LegSim {
                amount_in: a_in,
                amount_out: amount,
                pool_idxs: pools.clone(),
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
