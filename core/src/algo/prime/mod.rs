// PRIME: Pool-disjoint split routing via Generalised Network Flow.
//
// Implements the algorithm from the paper:
//
//   Stage 0 — Preprocessing: hub set, shortcut index, core-graph induction.
//   Stage 1 — Iterative path discovery: SPFA + dominance-pruned FindPath
//             admits paths whose flow-aware marginal price exceeds the
//             current ASGM equilibrium τ. Path-level ASGM after each.
//   Stage 2 — MergeAndExpand: paths sharing a token sequence merge into
//             one MultiEdgePath whose pools_at_hop[h] contains all parallel
//             pools at that hop. Final nested ASGM optimises both per-path
//             allocations and per-hop edge weights.
//
// Submodules:
//   config     — `PrimeConfig`, `AsgmConfig`.
//   hub        — `HubSet`, `CoreGraphView`, `RealEdge`, `build_hub_set`.
//   shortcut   — `Shortcut`, `ShortcutIndex`, `build_shortcut_index`.
//   path       — `DiscoveredPath`, `MultiEdgePath`.
//   sim        — multi-edge path simulation, marginal-price computations.
//   find_path  — Stage 1 SPFA.
//   merge      — Stage 2 MergeAndExpand.
//   asgm       — nested ASGM optimisation loop.
//   util       — shared U256 → f64 helper.

mod asgm;
pub mod config;
mod find_path;
pub mod hub;
mod merge;
pub mod path;
pub mod shortcut;
mod sim;
mod util;

use std::collections::HashSet;

use alloy_primitives::{Address, U256};

use crate::algo::amount_aware::MAX_HOPS as AMOUNT_AWARE_MAX_HOPS;
use crate::algo::bounded_bf::BoundedBfIter;
use crate::algo::gas::GasModel;
use crate::algo::path::walk_pool_path;
use crate::algo::{Leg, Outcome, SolveResult};
use crate::graph::Graph;
use crate::token::Token;
use crate::trace::Step;

pub use config::{AsgmConfig, PrimeConfig};
pub use find_path::find_path;
pub use hub::{CoreGraphView, HubSet, RealEdge, build_hub_set};
pub use merge::merge_and_expand;
pub use path::{DiscoveredPath, MultiEdgePath};
pub use shortcut::{Shortcut, ShortcutIndex, build_shortcut_index};

use asgm::run_asgm;
use sim::simulate_multi_edge_paths;

// Drop legs below 0.1% allocation — anything smaller is line-search noise.
const MIN_LEG_FRACTION: f64 = 0.001;

pub fn solve(
    graph: &Graph,
    src: Address,
    dst: Address,
    amount_in: U256,
    with_trace: bool,
    gas: &GasModel,
) -> SolveResult {
    solve_with_config(
        graph,
        src,
        dst,
        amount_in,
        with_trace,
        gas,
        &PrimeConfig::default(),
    )
}

pub fn solve_with_config(
    graph: &Graph,
    src: Address,
    dst: Address,
    amount_in: U256,
    with_trace: bool,
    gas: &GasModel,
    config: &PrimeConfig,
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

    // Stage 0: preprocessing.
    let hubs = build_hub_set(graph, config.hub_count);
    let shortcuts = build_shortcut_index(graph, &hubs, config);
    let core = CoreGraphView {
        graph,
        hubs: &hubs,
        s: src_idx,
        t: dst_idx,
    };

    // Seed: best single path on the full graph (also used for fallback).
    let Some((seed_tokens, seed_pools)) =
        best_single_path(graph, src_idx, dst_idx, amount_in, gas, dst_token)
    else {
        return no_path();
    };

    let mut used_pools: HashSet<usize> = seed_pools.iter().copied().collect();

    // Stage 1: iterative path discovery on the working set (single-pool-per-hop).
    let mut paths_v: Vec<MultiEdgePath> = vec![MultiEdgePath::from_single(
        seed_tokens.clone(),
        seed_pools.clone(),
        1.0,
    )];
    let mut tau = run_asgm(&mut paths_v, graph, amount_in, &config.asgm);

    for _ in 0..config.max_paths {
        let Some(disc) = find_path(
            graph,
            &core,
            &shortcuts,
            src_idx,
            dst_idx,
            amount_in,
            tau,
            &used_pools,
            config.max_hops,
        ) else {
            break;
        };
        for &p in &disc.pools {
            used_pools.insert(p);
        }
        paths_v.push(MultiEdgePath::from_single(disc.tokens, disc.pools, 0.0));
        tau = run_asgm(&mut paths_v, graph, amount_in, &config.asgm);
    }

    // Stage 2: MergeAndExpand + final nested ASGM.
    let merged_input: Vec<(MultiEdgePath, f64)> = paths_v
        .into_iter()
        .map(|p| {
            let a = p.alloc;
            (p, a)
        })
        .collect();
    let mut merged = merge_and_expand(merged_input, graph);
    let _tau_final = run_asgm(&mut merged, graph, amount_in, &config.asgm);

    // Final simulation + Leg emission.
    let (total_out, per_leg) = simulate_multi_edge_paths(graph, &merged, amount_in);

    let mut legs: Vec<Leg> = (0..merged.len())
        .filter_map(|i| {
            let sim = per_leg[i].clone()?;
            if merged[i].alloc < MIN_LEG_FRACTION {
                return None;
            }
            let path_addrs: Vec<Address> = merged[i]
                .tokens
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
        if let Some((bt, bp)) = best_single_path(graph, src_idx, dst_idx, amount_in, gas, dst_token)
            && let Some((out, _)) = walk_pool_path(graph, &bt, &bp, amount_in)
        {
            let gas_units = gas.gas_units(1, bp.len());
            let gas_cost =
                gas.gas_to_dst_token(gas_units, dst_token.true_price_usd, dst_token.decimals);
            return SolveResult {
                outcome: Outcome::FoundSplit {
                    legs: vec![Leg {
                        path: bt.iter().map(|&ix| graph.address_of(ix)).collect(),
                        pools_used: bp.iter().map(|&ix| graph.pools[ix].address).collect(),
                        amount_in,
                        amount_out: out,
                    }],
                    amount_in,
                    amount_out: out,
                    gas_cost,
                },
                trace: seed_trace(),
            };
        }
        return no_path();
    }

    let total_in: U256 = legs.iter().map(|l| l.amount_in).sum();
    legs.sort_by_key(|l| std::cmp::Reverse(l.amount_in));

    let total_hops: usize = legs.iter().map(|l| l.pools_used.len()).sum();
    let gas_units = gas.gas_units(legs.len(), total_hops);
    let gas_cost = gas.gas_to_dst_token(gas_units, dst_token.true_price_usd, dst_token.decimals);

    // Defensive seed-floor: if PRIME's final routing underperforms the
    // single-best path (rare with proper preprocessing), return the seed.
    let prime_net = total_out.saturating_sub(gas_cost);
    if let Some((seed_out, _)) = walk_pool_path(graph, &seed_tokens, &seed_pools, amount_in) {
        let seed_gas_units = gas.gas_units(1, seed_pools.len());
        let seed_gas =
            gas.gas_to_dst_token(seed_gas_units, dst_token.true_price_usd, dst_token.decimals);
        let seed_net = seed_out.saturating_sub(seed_gas);
        if seed_net > prime_net {
            return SolveResult {
                outcome: Outcome::FoundSplit {
                    legs: vec![Leg {
                        path: seed_tokens.iter().map(|&ix| graph.address_of(ix)).collect(),
                        pools_used: seed_pools
                            .iter()
                            .map(|&ix| graph.pools[ix].address)
                            .collect(),
                        amount_in,
                        amount_out: seed_out,
                    }],
                    amount_in,
                    amount_out: seed_out,
                    gas_cost: seed_gas,
                },
                trace: seed_trace(),
            };
        }
    }

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

// Best (token-path, pool-path) pair by gas-net realised output at amount_in.
// Capped at 200 candidates from BoundedBfIter — matches V1 behaviour and
// keeps cold-seed cost bounded on big graphs.
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
    for cand in BoundedBfIter::new(graph, src_idx, dst_idx, AMOUNT_AWARE_MAX_HOPS) {
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
        if examined >= 200 {
            break;
        }
    }
    best
}

fn no_path() -> SolveResult {
    SolveResult {
        outcome: Outcome::NoPath,
        trace: Vec::new(),
    }
}

#[cfg(test)]
mod tests;
