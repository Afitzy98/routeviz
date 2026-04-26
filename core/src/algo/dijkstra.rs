use std::cmp::Reverse;
use std::collections::BinaryHeap;

use alloy_primitives::{Address, U256};
use ordered_float::OrderedFloat;

use crate::algo::gas::GasModel;
use crate::algo::path::{build_by_pair, reconstruct_with_pools, walk_with_best_pools};
use crate::algo::{Outcome, SolveResult, Tracer};
use crate::graph::Graph;
use crate::trace::Step;

// Log-weighted directed Dijkstra. Edge weight = -ln(marginal_rate),
// which can be negative — Dijkstra is only correct here under uniform
// fees and a near-arbitrage-free graph (a price potential exists that
// makes reduced weights non-negative). With variable fees or real
// arbs, switch to Bellman-Ford or Johnson's. Path is chosen in
// log-space; the reported amount_out comes from exact U256 simulate.
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

    let n = graph.num_tokens();
    let mut dist: Vec<f64> = vec![f64::INFINITY; n];
    // prev[node] = Some((parent, pool_idx)).
    let mut prev: Vec<Option<(usize, usize)>> = vec![None; n];
    let mut visited = vec![false; n];
    let mut tracer = Tracer::new(with_trace);

    dist[src_idx] = 0.0;
    let mut heap: BinaryHeap<Reverse<(OrderedFloat<f64>, usize)>> = BinaryHeap::new();
    heap.push(Reverse((OrderedFloat(0.0), src_idx)));

    while let Some(Reverse((OrderedFloat(d), u))) = heap.pop() {
        if visited[u] {
            continue;
        }
        visited[u] = true;
        tracer.push(Step::Visit(u));

        if u == dst_idx {
            break;
        }

        for edge in &graph.adj[u] {
            if visited[edge.to] {
                continue;
            }
            let w = graph.pools[edge.pool].log_weight(edge.in_token);
            if !w.is_finite() {
                continue;
            }
            let new_dist = d + w;
            if new_dist < dist[edge.to] {
                dist[edge.to] = new_dist;
                prev[edge.to] = Some((u, edge.pool));
                tracer.push(Step::Relax {
                    from: u,
                    to: edge.to,
                    new_distance: new_dist,
                });
                heap.push(Reverse((OrderedFloat(new_dist), edge.to)));
            }
        }
    }

    if !visited[dst_idx] || dist[dst_idx].is_infinite() {
        return SolveResult {
            outcome: Outcome::NoPath,
            trace: tracer.into_vec(),
        };
    }

    let (path_idx, _) = reconstruct_with_pools(&prev, src_idx, dst_idx);
    let path_addresses: Vec<Address> = path_idx.iter().map(|&i| graph.address_of(i)).collect();

    // Re-pick pool per hop at the user's trade size: marginal rate and
    // realised output disagree on parallel pools of differing depth.
    let by_pair = build_by_pair(graph);
    let (pools_used_idx, amount_out, total_log_weight) =
        match walk_with_best_pools(graph, &by_pair, &path_idx, amount_in) {
            Some(t) => t,
            None => {
                return SolveResult {
                    outcome: Outcome::NoPath,
                    trace: tracer.into_vec(),
                };
            }
        };
    let pools_used_addresses: Vec<Address> = pools_used_idx
        .iter()
        .map(|&i| graph.pools[i].address)
        .collect();
    let product_of_rates = (-total_log_weight).exp();

    // Reported gas; Dijkstra doesn't optimise net of it (single path).
    let dst_token = &graph.tokens[dst_idx];
    let gas_units = gas.gas_units(1, pools_used_addresses.len());
    let gas_cost = gas.gas_to_dst_token(gas_units, dst_token.true_price_usd, dst_token.decimals);

    SolveResult {
        outcome: Outcome::Found {
            path: path_addresses,
            pools_used: pools_used_addresses,
            total_log_weight,
            product_of_rates,
            amount_in,
            amount_out,
            gas_cost,
        },
        trace: tracer.into_vec(),
    }
}

#[cfg(test)]
mod tests {
    use crate::algo::Algorithm;
    use crate::algo::testkit;

    #[test]
    fn src_equals_dst_returns_singleton() {
        testkit::src_equals_dst_returns_singleton(Algorithm::Dijkstra);
    }

    #[test]
    fn two_nodes_one_edge() {
        testkit::two_nodes_one_edge(Algorithm::Dijkstra);
    }

    #[test]
    fn no_path_when_disconnected() {
        testkit::no_path_when_disconnected(Algorithm::Dijkstra);
    }

    #[test]
    fn unknown_source_or_dst_returns_no_path() {
        testkit::unknown_source_or_dst_returns_no_path(Algorithm::Dijkstra);
    }

    #[test]
    fn path_is_contiguous_via_real_edges() {
        testkit::path_is_contiguous_via_real_edges(Algorithm::Dijkstra);
    }

    #[test]
    fn reported_log_weight_equals_sum_of_hops() {
        testkit::reported_log_weight_equals_sum_of_hops(Algorithm::Dijkstra);
    }

    #[test]
    fn trace_is_nonempty_for_reachable_dst() {
        testkit::trace_is_nonempty_for_reachable_dst(Algorithm::Dijkstra);
    }

    #[test]
    fn every_trace_event_references_real_nodes() {
        testkit::every_trace_event_references_real_nodes(Algorithm::Dijkstra);
    }

    #[test]
    fn prefers_cheaper_pool_on_same_pair() {
        testkit::prefers_cheaper_pool_on_same_pair(Algorithm::Dijkstra);
    }

    #[test]
    fn amount_out_matches_simulate_path() {
        testkit::amount_out_matches_simulate_path(Algorithm::Dijkstra);
    }

    #[test]
    fn product_of_rates_equals_exp_neg_total_log_weight() {
        testkit::product_of_rates_equals_exp_neg_total_log_weight(Algorithm::Dijkstra);
    }

    #[test]
    fn solve_on_generated_graph_returns_valid_path() {
        testkit::solve_on_generated_graph_returns_valid_path(Algorithm::Dijkstra);
    }
}
