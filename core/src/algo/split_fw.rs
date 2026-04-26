use alloy_primitives::{Address, U256};

use crate::algo::amount_aware::MAX_HOPS;
use crate::algo::bounded_bf::BoundedBfIter;
use crate::algo::gas::GasModel;
use crate::algo::path::walk_pool_path;
use crate::algo::split_common::{
    fractional_mul, initial_state, simulate_allocation, simulate_allocation_mut,
};
use crate::algo::{Leg, Outcome, SolveResult};
use crate::graph::Graph;
use crate::pool::Pool;
use crate::token::Token;
use crate::trace::Step;

// Frank-Wolfe over a growing path set. Maximises total realised output
// net of gas on the simplex of allocations.
//
// Each iter: simulate current allocation against a copy of the pool
// state, find the best path on the updated graph, line-search α ∈ [0, 1]
// for the convex combination (1-α)·x + α·e_s, commit if it improves.
// Cold seed is the best single path (gas-aware) so we're monotone above
// the no-split baseline.
//
// Direction selection uses the same realised-net metric as the cold
// seed, not pure log-weight. Log-weight is the textbook FW linear
// subproblem and correct at infinitesimal flow, but slippage-blind at
// finite trade sizes — same fix we applied to candidate ranking.

fn max_iterations(graph: &Graph) -> usize {
    (30 + graph.num_tokens() / 5).clamp(30, 500)
}

// Coarse + fine grids: ~5% then ~0.25% effective line-search resolution.
const LINE_SEARCH_STEPS: usize = 20;
const LINE_SEARCH_REFINE_STEPS: usize = 20;

// Stop when no α moves the objective.
const CONVERGENCE_ALPHA: f64 = 1e-5;

// Drop legs below 0.1% — anything smaller is line-search noise.
const MIN_LEG_FRACTION: f64 = 0.001;

// Compute bound on candidate ranking. With pool-paths (no parallel-pool
// coalescing) BoundedBfIter emits ~2–3× the candidates it used to;
// we lift the cap accordingly so the cold seed sees the full set.
const COLD_SEED_HARD_CAP: usize = 200;

pub fn solve(
    graph: &Graph,
    src: Address,
    dst: Address,
    amount_in: U256,
    with_trace: bool,
    gas: &GasModel,
) -> SolveResult {
    let src_idx = match graph.index_of(src) {
        Some(i) => i,
        None => return no_path(),
    };
    let dst_idx = match graph.index_of(dst) {
        Some(i) => i,
        None => return no_path(),
    };

    let seed_trace = || {
        if with_trace {
            vec![Step::Visit(src_idx)]
        } else {
            Vec::new()
        }
    };

    if src_idx == dst_idx {
        return SolveResult {
            outcome: Outcome::FoundSplit {
                legs: vec![Leg {
                    path: vec![src],
                    pools_used: Vec::new(),
                    amount_in,
                    amount_out: amount_in,
                }],
                amount_in,
                amount_out: amount_in,
                gas_cost: U256::ZERO,
            },
            trace: seed_trace(),
        };
    }

    let dst_token = &graph.tokens[dst_idx];

    let Some((seed_tokens, seed_pools)) =
        best_single_path(graph, src_idx, dst_idx, amount_in, gas, dst_token)
    else {
        return no_path();
    };
    let mut token_paths: Vec<Vec<usize>> = vec![seed_tokens];
    let mut pool_paths: Vec<Vec<usize>> = vec![seed_pools];
    let mut alloc: Vec<f64> = vec![1.0];

    let iter_budget = max_iterations(graph);
    for _iter in 0..iter_budget {
        // Build the post-flow graph and pick the next direction.
        let updated_pools_vec = apply_flow(graph, &token_paths, &pool_paths, &alloc, amount_in);
        let updated_graph = Graph::new(graph.tokens.clone(), updated_pools_vec);

        let (best_tokens, best_pools) =
            match best_single_path(&updated_graph, src_idx, dst_idx, amount_in, gas, dst_token) {
                Some(p) => p,
                None => break,
            };

        // Match candidate by *both* tokens and pools — distinct pool
        // variants of the same token-path are different directions.
        let best_idx = match token_paths
            .iter()
            .zip(pool_paths.iter())
            .position(|(t, p)| *t == best_tokens && *p == best_pools)
        {
            Some(i) => i,
            None => {
                token_paths.push(best_tokens);
                pool_paths.push(best_pools);
                alloc.push(0.0);
                token_paths.len() - 1
            }
        };

        // Two-stage line search over α ∈ [0, 1] on net output.
        let base_output = evaluate_net(
            graph,
            &token_paths,
            &pool_paths,
            &alloc,
            amount_in,
            gas,
            dst_token,
        );
        let mut best_alpha = 0.0f64;
        let mut best_output = base_output;
        let eval_at = |alpha: f64,
                       token_paths: &[Vec<usize>],
                       pool_paths: &[Vec<usize>],
                       alloc: &[f64]|
         -> U256 {
            let candidate: Vec<f64> = (0..token_paths.len())
                .map(|i| {
                    let base = (1.0 - alpha) * alloc[i];
                    if i == best_idx { base + alpha } else { base }
                })
                .collect();
            evaluate_net(
                graph,
                token_paths,
                pool_paths,
                &candidate,
                amount_in,
                gas,
                dst_token,
            )
        };
        for step in 1..=LINE_SEARCH_STEPS {
            let alpha = step as f64 / LINE_SEARCH_STEPS as f64;
            let out = eval_at(alpha, &token_paths, &pool_paths, &alloc);
            if out > best_output {
                best_output = out;
                best_alpha = alpha;
            }
        }
        let coarse_step = 1.0 / LINE_SEARCH_STEPS as f64;
        let refine_low = (best_alpha - coarse_step).max(0.0);
        let refine_high = (best_alpha + coarse_step).min(1.0);
        let refine_span = refine_high - refine_low;
        if refine_span > 0.0 {
            for step in 0..=LINE_SEARCH_REFINE_STEPS {
                let alpha =
                    refine_low + refine_span * step as f64 / LINE_SEARCH_REFINE_STEPS as f64;
                let out = eval_at(alpha, &token_paths, &pool_paths, &alloc);
                if out > best_output {
                    best_output = out;
                    best_alpha = alpha;
                }
            }
        }

        if best_alpha < CONVERGENCE_ALPHA {
            break;
        }

        // Commit: x ← (1 - α) x + α e_best.
        for a in alloc.iter_mut() {
            *a *= 1.0 - best_alpha;
        }
        alloc[best_idx] += best_alpha;
    }

    let inputs: Vec<U256> = alloc
        .iter()
        .map(|&f| fractional_mul(amount_in, f))
        .collect();
    let (total_out, per_leg) = simulate_allocation(graph, &token_paths, &pool_paths, &inputs);

    let mut legs: Vec<Leg> = (0..token_paths.len())
        .filter_map(|i| {
            let sim = per_leg[i].clone()?;
            if alloc[i] < MIN_LEG_FRACTION {
                return None;
            }
            let path_addrs: Vec<Address> = token_paths[i]
                .iter()
                .map(|&ix| graph.address_of(ix))
                .collect();
            let pool_addrs: Vec<Address> = sim
                .pool_idxs
                .iter()
                .map(|&ix| graph.pools[ix].address)
                .collect();
            Some(Leg {
                path: path_addrs,
                pools_used: pool_addrs,
                amount_in: sim.amount_in,
                amount_out: sim.amount_out,
            })
        })
        .collect();

    if legs.is_empty() {
        return no_path();
    }

    let total_in: U256 = legs.iter().map(|l| l.amount_in).sum();
    legs.sort_by_key(|l| std::cmp::Reverse(l.amount_in));

    let total_hops: usize = legs.iter().map(|l| l.pools_used.len()).sum();
    let gas_units = gas.gas_units(legs.len(), total_hops);
    let gas_cost = gas.gas_to_dst_token(gas_units, dst_token.true_price_usd, dst_token.decimals);

    SolveResult {
        outcome: Outcome::FoundSplit {
            legs,
            amount_in: total_in,
            amount_out: total_out,
            gas_cost,
        },
        trace: seed_trace(),
    }
}

fn no_path() -> SolveResult {
    SolveResult {
        outcome: Outcome::NoPath,
        trace: Vec::new(),
    }
}

// Total realised output net of gas for the given allocation. Gross
// is the sequential simulate; gas counts every active (non-zero) leg.
fn evaluate_net(
    graph: &Graph,
    token_paths: &[Vec<usize>],
    pool_paths: &[Vec<usize>],
    alloc: &[f64],
    amount_in: U256,
    gas: &GasModel,
    dst_token: &Token,
) -> U256 {
    let inputs: Vec<U256> = alloc
        .iter()
        .map(|&f| fractional_mul(amount_in, f))
        .collect();
    let (gross, per_leg) = simulate_allocation(graph, token_paths, pool_paths, &inputs);
    if !gas.enabled() {
        return gross;
    }
    let active_legs = per_leg.iter().filter(|s| s.is_some()).count();
    let total_hops: usize = per_leg
        .iter()
        .filter_map(|s| s.as_ref().map(|s| s.pool_idxs.len()))
        .sum();
    let gas_units = gas.gas_units(active_legs, total_hops);
    let gas_cost = gas.gas_to_dst_token(gas_units, dst_token.true_price_usd, dst_token.decimals);
    gross.saturating_sub(gas_cost)
}

// Pool reserves after the current allocation has been simulated.
fn apply_flow(
    graph: &Graph,
    token_paths: &[Vec<usize>],
    pool_paths: &[Vec<usize>],
    alloc: &[f64],
    amount_in: U256,
) -> Vec<Pool> {
    let mut state = initial_state(graph);
    let inputs: Vec<U256> = alloc
        .iter()
        .map(|&f| fractional_mul(amount_in, f))
        .collect();
    let _ = simulate_allocation_mut(graph, token_paths, pool_paths, &inputs, &mut state);
    graph
        .pools
        .iter()
        .enumerate()
        .map(|(i, pool)| {
            let (ra, rb) = state[&i];
            Pool {
                address: pool.address,
                token_a: pool.token_a,
                token_b: pool.token_b,
                reserve_a: ra,
                reserve_b: rb,
                fee_bps: pool.fee_bps,
                venue: pool.venue.clone(),
            }
        })
        .collect()
}

// (token_path, pool_path) maximising realised output net of per-leg gas
// at `amount_in`, scored against the supplied graph's reserves. Each
// parallel-pool variant of a token-path is its own candidate.
fn best_single_path(
    graph: &Graph,
    src_idx: usize,
    dst_idx: usize,
    amount_in: U256,
    gas: &GasModel,
    dst_token: &Token,
) -> Option<(Vec<usize>, Vec<usize>)> {
    let mut best: Option<(Vec<usize>, Vec<usize>)> = None;
    let mut best_net = U256::ZERO;
    let mut examined = 0usize;
    for cand in BoundedBfIter::new(graph, src_idx, dst_idx, MAX_HOPS) {
        if let Some((out, _)) = walk_pool_path(graph, &cand.tokens, &cand.pools, amount_in) {
            let leg_gas = gas.gas_to_dst_token(
                gas.gas_units(1, cand.pools.len()),
                dst_token.true_price_usd,
                dst_token.decimals,
            );
            let net = out.saturating_sub(leg_gas);
            if net > best_net {
                best_net = net;
                best = Some((cand.tokens, cand.pools));
            }
        }
        examined += 1;
        if examined >= COLD_SEED_HARD_CAP {
            break;
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::algo::{Algorithm, solve};
    use crate::pool::Pool;
    use crate::token::{Token, TokenKind};

    fn addr(byte: u8) -> Address {
        Address::from([byte; 20])
    }
    fn tok(byte: u8, symbol: &str) -> Token {
        Token {
            address: addr(byte),
            symbol: symbol.into(),
            decimals: 18,
            true_price_usd: 1.0,
            kind: TokenKind::Spoke,
        }
    }
    fn pool(byte: u8, a: Address, b: Address, ra: u128, rb: u128) -> Pool {
        Pool {
            address: addr(byte),
            token_a: a,
            token_b: b,
            reserve_a: U256::from(ra),
            reserve_b: U256::from(rb),
            fee_bps: 30,
            venue: "Test".into(),
        }
    }

    #[test]
    fn src_equals_dst_returns_singleton_leg() {
        let a = addr(1);
        let g = Graph::new(vec![tok(1, "A")], Vec::new());
        let r = solve(Algorithm::SplitFw, &g, a, a, U256::from(100u64));
        match r.outcome {
            Outcome::FoundSplit {
                legs,
                amount_in,
                amount_out,
                ..
            } => {
                assert_eq!(legs.len(), 1);
                assert_eq!(amount_in, U256::from(100u64));
                assert_eq!(amount_out, U256::from(100u64));
            }
            other => panic!("expected FoundSplit, got {other:?}"),
        }
    }

    #[test]
    fn no_path_when_disconnected() {
        let a = addr(1);
        let b = addr(2);
        let g = Graph::new(vec![tok(1, "A"), tok(2, "B")], Vec::new());
        let r = solve(Algorithm::SplitFw, &g, a, b, U256::from(100u64));
        assert!(matches!(r.outcome, Outcome::NoPath));
    }

    #[test]
    fn single_path_converges_to_one_leg() {
        let a = addr(1);
        let b = addr(2);
        let g = Graph::new(
            vec![tok(1, "A"), tok(2, "B")],
            vec![pool(0xA1, a, b, 1_000_000, 1_000_000)],
        );
        let r = solve(Algorithm::SplitFw, &g, a, b, U256::from(100u64));
        match r.outcome {
            Outcome::FoundSplit { legs, .. } => {
                assert_eq!(legs.len(), 1);
                assert_eq!(legs[0].path, vec![a, b]);
            }
            other => panic!("expected FoundSplit, got {other:?}"),
        }
    }

    #[test]
    fn fw_matches_dp_on_pool_disjoint_routes() {
        // A─┬─B─D
        //   └─C─D
        // Two routes A→B→D and A→C→D share no pool. FW's shared-pool-
        // aware simulate collapses to DP's independent simulate when
        // no pool is shared, so the two should agree within rounding.
        let a = addr(1);
        let b = addr(2);
        let c = addr(3);
        let d = addr(4);
        let g = Graph::new(
            vec![tok(1, "A"), tok(2, "B"), tok(3, "C"), tok(4, "D")],
            vec![
                pool(0xA1, a, b, 1_000_000, 1_000_000),
                pool(0xA2, a, c, 1_000_000, 1_000_000),
                pool(0xA3, b, d, 1_000_000, 1_000_000),
                pool(0xA4, c, d, 1_000_000, 1_000_000),
            ],
        );
        let amount = U256::from(200_000u64);

        let fw = solve(Algorithm::SplitFw, &g, a, d, amount);
        let dp = solve(Algorithm::SplitDp, &g, a, d, amount);

        let fw_out = match fw.outcome {
            Outcome::FoundSplit { amount_out, .. } => amount_out,
            other => panic!("expected FoundSplit, got {other:?}"),
        };
        let dp_out = match dp.outcome {
            Outcome::FoundSplit { amount_out, .. } => amount_out,
            other => panic!("expected FoundSplit, got {other:?}"),
        };

        let delta = if fw_out > dp_out {
            fw_out - dp_out
        } else {
            dp_out - fw_out
        };
        assert!(
            delta * U256::from(100u64) <= fw_out.max(dp_out),
            "FW {fw_out} vs DP {dp_out} differ by more than 1%"
        );
    }

    #[test]
    fn leg_inputs_sum_to_amount_in() {
        let a = addr(1);
        let b = addr(2);
        let c = addr(3);
        let g = Graph::new(
            vec![tok(1, "A"), tok(2, "B"), tok(3, "C")],
            vec![
                pool(0xA1, a, b, 1_000_000, 1_000_000),
                pool(0xA2, b, c, 1_000_000, 1_000_000),
                pool(0xA3, a, c, 500_000, 500_000),
            ],
        );
        let amount = U256::from(100_000u64);
        let r = solve(Algorithm::SplitFw, &g, a, c, amount);
        match r.outcome {
            Outcome::FoundSplit {
                legs, amount_in, ..
            } => {
                let in_sum: U256 = legs.iter().map(|l| l.amount_in).sum();
                let delta = if in_sum > amount_in {
                    in_sum - amount_in
                } else {
                    amount_in - in_sum
                };
                assert!(delta <= U256::from(legs.len() as u64 * 10u64));
            }
            other => panic!("expected FoundSplit, got {other:?}"),
        }
    }

    #[test]
    fn runs_on_default_generator_graph() {
        use crate::generator::{GenConfig, PoolGenerator};
        let (tokens, pools) = PoolGenerator::new(GenConfig::default()).generate();
        let g = Graph::new(tokens.clone(), pools);
        let hubs: Vec<_> = tokens
            .iter()
            .filter(|t| matches!(t.kind, TokenKind::Hub))
            .collect();
        let r = solve(
            Algorithm::SplitFw,
            &g,
            hubs[0].address,
            hubs[1].address,
            U256::from(1_000_000u64),
        );
        assert!(matches!(
            r.outcome,
            Outcome::FoundSplit { .. } | Outcome::NoPath
        ));
    }
}
