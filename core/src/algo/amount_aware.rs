use alloy_primitives::{Address, U256};

use crate::algo::bounded_bf::BoundedBfIter;
use crate::algo::gas::GasModel;
use crate::algo::path::walk_pool_path;
use crate::algo::{Outcome, SolveResult, Tracer};
use crate::graph::Graph;
use crate::trace::Step;

// Single-path routing. Enumerate (token-path, pool-path) candidates via
// `BoundedBfIter`, score each by realised output net of leg gas at the
// user's trade size, take the max. Each parallel pool variant is its
// own candidate, so we naturally consider `[USDC→USDT via UniV2]` and
// `[USDC→USDT via Sushi]` as distinct options.

pub const MAX_HOPS: usize = 3;

const HARD_CAP: usize = 200;

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
        None => {
            return SolveResult {
                outcome: Outcome::NoPath,
                trace: Vec::new(),
            };
        }
    };
    let dst_idx = match graph.index_of(dst) {
        Some(i) => i,
        None => {
            return SolveResult {
                outcome: Outcome::NoPath,
                trace: Vec::new(),
            };
        }
    };

    if src_idx == dst_idx {
        let mut tracer = Tracer::new(with_trace);
        tracer.push(Step::Visit(src_idx));
        return SolveResult {
            outcome: Outcome::Found {
                path: vec![src],
                pools_used: Vec::new(),
                total_log_weight: 0.0,
                product_of_rates: 1.0,
                amount_in,
                amount_out: amount_in,
                gas_cost: U256::ZERO,
            },
            trace: tracer.into_vec(),
        };
    }

    let dst_token = &graph.tokens[dst_idx];

    let mut best_net = U256::ZERO;
    let mut best_out = U256::ZERO;
    let mut best_path_idx: Option<Vec<usize>> = None;
    let mut best_pools_idx: Option<Vec<usize>> = None;
    let mut best_log_weight = 0.0f64;

    let mut tracer = Tracer::new(with_trace);
    tracer.push(Step::Visit(src_idx));
    let mut examined = 0usize;

    let candidates = BoundedBfIter::new(graph, src_idx, dst_idx, MAX_HOPS);

    for cand in candidates {
        if let Some((output, log_weight)) =
            walk_pool_path(graph, &cand.tokens, &cand.pools, amount_in)
        {
            let gas_units = gas.gas_units(1, cand.pools.len());
            let leg_gas =
                gas.gas_to_dst_token(gas_units, dst_token.true_price_usd, dst_token.decimals);
            let net = output.saturating_sub(leg_gas);
            if net > best_net {
                best_net = net;
                best_out = output;
                best_path_idx = Some(cand.tokens);
                best_pools_idx = Some(cand.pools);
                best_log_weight = log_weight;
            }
        }
        examined += 1;
        if examined >= HARD_CAP {
            break;
        }
    }

    // Replay the winning path so the canvas animation ends correctly.
    if let (Some(path_idx), Some(pools_idx)) = (&best_path_idx, &best_pools_idx) {
        let mut running = 0.0f64;
        for hop in 0..path_idx.len().saturating_sub(1) {
            let pool_idx = pools_idx[hop];
            let from_addr = graph.tokens[path_idx[hop]].address;
            running += graph.pools[pool_idx].log_weight(from_addr);
            tracer.push(Step::Relax {
                from: path_idx[hop],
                to: path_idx[hop + 1],
                new_distance: running,
            });
            tracer.push(Step::Visit(path_idx[hop + 1]));
        }
    }

    match (best_path_idx, best_pools_idx) {
        (Some(path_idx), Some(pools_idx)) => {
            let path: Vec<Address> = path_idx.iter().map(|&i| graph.address_of(i)).collect();
            let pools_used: Vec<Address> =
                pools_idx.iter().map(|&i| graph.pools[i].address).collect();
            let product_of_rates = (-best_log_weight).exp();
            let gas_units = gas.gas_units(1, pools_used.len());
            let gas_cost =
                gas.gas_to_dst_token(gas_units, dst_token.true_price_usd, dst_token.decimals);
            SolveResult {
                outcome: Outcome::Found {
                    path,
                    pools_used,
                    total_log_weight: best_log_weight,
                    product_of_rates,
                    amount_in,
                    amount_out: best_out,
                    gas_cost,
                },
                trace: tracer.into_vec(),
            }
        }
        _ => SolveResult {
            outcome: Outcome::NoPath,
            trace: tracer.into_vec(),
        },
    }
}

#[cfg(test)]
mod tests {
    use crate::algo::Algorithm;
    use crate::algo::testkit;

    #[test]
    fn src_equals_dst_returns_singleton() {
        testkit::src_equals_dst_returns_singleton(Algorithm::AmountAware);
    }

    #[test]
    fn two_nodes_one_edge() {
        testkit::two_nodes_one_edge(Algorithm::AmountAware);
    }

    #[test]
    fn no_path_when_disconnected() {
        testkit::no_path_when_disconnected(Algorithm::AmountAware);
    }

    #[test]
    fn unknown_source_or_dst_returns_no_path() {
        testkit::unknown_source_or_dst_returns_no_path(Algorithm::AmountAware);
    }

    #[test]
    fn path_is_contiguous_via_real_edges() {
        testkit::path_is_contiguous_via_real_edges(Algorithm::AmountAware);
    }

    #[test]
    fn prefers_cheaper_pool_on_same_pair() {
        testkit::prefers_cheaper_pool_on_same_pair(Algorithm::AmountAware);
    }

    #[test]
    fn amount_out_matches_simulate_path() {
        testkit::amount_out_matches_simulate_path(Algorithm::AmountAware);
    }

    #[test]
    fn picks_direct_over_multi_hop_when_direct_wins_with_slippage() {
        testkit::amount_aware_prefers_direct_when_direct_has_better_liquidity();
    }

    #[test]
    fn picks_multi_hop_for_small_trade_through_same_graph() {
        testkit::amount_aware_picks_multi_hop_for_small_trade();
    }

    #[test]
    fn runs_on_default_generator_graph() {
        // Default config has price_noise = 0.01 → real negative cycles.
        // Smoke test: must terminate, must not panic.
        use crate::algo::Algorithm;
        use crate::algo::solve;
        use crate::generator::{GenConfig, PoolGenerator};
        use crate::graph::Graph;
        use crate::token::TokenKind;
        use alloy_primitives::U256;

        let (tokens, pools) = PoolGenerator::new(GenConfig::default()).generate();
        let g = Graph::new(tokens.clone(), pools);
        let hubs: Vec<_> = tokens
            .iter()
            .filter(|t| matches!(t.kind, TokenKind::Hub))
            .collect();
        let r = solve(
            Algorithm::AmountAware,
            &g,
            hubs[0].address,
            hubs[1].address,
            U256::from(1_000_000u64),
        );
        let _ = r.outcome;
    }
}
