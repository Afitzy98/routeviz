use alloy_primitives::{Address, U256};

use crate::algo::gas::GasModel;
use crate::algo::path::{build_by_pair, reconstruct_with_pools, walk_with_best_pools};
use crate::algo::{Outcome, SolveResult, Tracer};
use crate::graph::Graph;
use crate::pool::Pool;
use crate::trace::Step;

// Tolerance for relaxation. Float log/exp round-trips can leave 1e-15
// noise on zero-fee cycles; the smallest real gap (1 bps fee drag) is
// ~1e-4, so 1e-10 has plenty of headroom.
const RELAX_EPSILON: f64 = 1e-10;

// Bellman-Ford. Correct on negative edges and reachable negative cycles
// — used both for arb-aware src→dst routing and for cycle detection.
// A reachable negative cycle takes priority over any src→dst path.
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
    let dst_idx_opt = graph.index_of(dst);

    if Some(src_idx) == dst_idx_opt {
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
    let mut prev: Vec<Option<(usize, usize)>> = vec![None; n];
    let mut tracer = Tracer::new(with_trace);
    dist[src_idx] = 0.0;

    // V-1 passes; early-exit when a sweep produces no updates.
    let max_passes = n.saturating_sub(1);
    for pass in 0..max_passes {
        tracer.push(Step::Pass(pass));
        let mut any_relaxed = false;
        for u in 0..n {
            if dist[u].is_infinite() {
                continue;
            }
            for edge in &graph.adj[u] {
                let w = graph.pools[edge.pool].log_weight(edge.in_token);
                if !w.is_finite() {
                    continue;
                }
                let new_dist = dist[u] + w;
                if new_dist + RELAX_EPSILON < dist[edge.to] {
                    dist[edge.to] = new_dist;
                    prev[edge.to] = Some((u, edge.pool));
                    tracer.push(Step::Relax {
                        from: u,
                        to: edge.to,
                        new_distance: new_dist,
                    });
                    any_relaxed = true;
                }
            }
        }
        if !any_relaxed {
            break;
        }
    }

    // V-th sweep: any still-relaxable edge proves a reachable cycle.
    for u in 0..n {
        if dist[u].is_infinite() {
            continue;
        }
        for edge in &graph.adj[u] {
            let w = graph.pools[edge.pool].log_weight(edge.in_token);
            if !w.is_finite() {
                continue;
            }
            if dist[u] + w + RELAX_EPSILON < dist[edge.to] {
                prev[edge.to] = Some((u, edge.pool));
                tracer.push(Step::Relax {
                    from: u,
                    to: edge.to,
                    new_distance: dist[u] + w,
                });
                return extract_cycle(graph, &prev, edge.to, amount_in, gas, tracer.into_vec());
            }
        }
    }

    // No cycle — plain shortest path.
    let dst_idx = match dst_idx_opt {
        Some(i) => i,
        None => {
            return SolveResult {
                outcome: Outcome::NoPath,
                trace: tracer.into_vec(),
            };
        }
    };
    if dist[dst_idx].is_infinite() {
        return SolveResult {
            outcome: Outcome::NoPath,
            trace: tracer.into_vec(),
        };
    }

    let (path_idx, _) = reconstruct_with_pools(&prev, src_idx, dst_idx);
    let path_addresses: Vec<Address> = path_idx.iter().map(|&i| graph.address_of(i)).collect();

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

// Walk `prev` V times from `entry` (pigeonhole guarantees we land on
// the cycle), then walk again to close the loop.
fn extract_cycle(
    graph: &Graph,
    prev: &[Option<(usize, usize)>],
    entry: usize,
    amount_in: U256,
    gas: &GasModel,
    trace: Vec<Step>,
) -> SolveResult {
    let n = graph.num_tokens();

    let mut current = entry;
    for _ in 0..n {
        match prev[current] {
            Some((parent, _)) => current = parent,
            None => {
                return SolveResult {
                    outcome: Outcome::NoPath,
                    trace,
                };
            }
        }
    }

    let cycle_start = current;
    // Collect (source, pool) hops by walking `prev` back to cycle_start.
    // Reversed at the end so pairs[i] = forward hop i.
    let mut backward: Vec<(usize, usize)> = Vec::new();
    loop {
        let (parent, pool_idx) = match prev[current] {
            Some(pair) => pair,
            None => {
                return SolveResult {
                    outcome: Outcome::NoPath,
                    trace,
                };
            }
        };
        backward.push((parent, pool_idx));
        if parent == cycle_start {
            break;
        }
        current = parent;
    }
    backward.reverse();
    let (cycle_nodes, cycle_pools): (Vec<usize>, Vec<usize>) = backward.into_iter().unzip();

    // Log-weight sum around the cycle. By BF's correctness this sum is
    // negative, i.e. product_of_rates > 1.
    let cycle_log_weight: f64 = (0..cycle_pools.len())
        .map(|i| {
            let pool = &graph.pools[cycle_pools[i]];
            let in_token = graph.address_of(cycle_nodes[i]);
            pool.log_weight(in_token)
        })
        .sum();
    let product_of_rates = (-cycle_log_weight).exp();

    // simulate_path expects hops+1 tokens — append the start to close.
    let cycle_addresses: Vec<Address> = cycle_nodes.iter().map(|&i| graph.address_of(i)).collect();
    let mut simulate_tokens: Vec<Address> = cycle_addresses.clone();
    simulate_tokens.push(cycle_addresses[0]);
    let cycle_pool_addresses: Vec<Address> = cycle_pools
        .iter()
        .map(|&i| graph.pools[i].address)
        .collect();

    let cycle_output = Pool::simulate_path(
        &simulate_tokens,
        &cycle_pool_addresses,
        &graph.pools,
        amount_in,
    );

    // Gas charged in the entry/exit token (cycles close on themselves).
    let entry_token = &graph.tokens[cycle_nodes[0]];
    let gas_units = gas.gas_units(1, cycle_pool_addresses.len());
    let gas_cost =
        gas.gas_to_dst_token(gas_units, entry_token.true_price_usd, entry_token.decimals);

    SolveResult {
        outcome: Outcome::NegativeCycle {
            cycle: cycle_addresses,
            pools_used: cycle_pool_addresses,
            product_of_rates,
            amount_in,
            cycle_output,
            gas_cost,
        },
        trace,
    }
}

#[cfg(test)]
mod tests {
    use crate::algo::Algorithm;
    use crate::algo::testkit;

    #[test]
    fn src_equals_dst_returns_singleton() {
        testkit::src_equals_dst_returns_singleton(Algorithm::BellmanFord);
    }

    #[test]
    fn two_nodes_one_edge() {
        testkit::two_nodes_one_edge(Algorithm::BellmanFord);
    }

    #[test]
    fn no_path_when_disconnected() {
        testkit::no_path_when_disconnected(Algorithm::BellmanFord);
    }

    #[test]
    fn unknown_source_or_dst_returns_no_path() {
        testkit::unknown_source_or_dst_returns_no_path(Algorithm::BellmanFord);
    }

    #[test]
    fn path_is_contiguous_via_real_edges() {
        testkit::path_is_contiguous_via_real_edges(Algorithm::BellmanFord);
    }

    #[test]
    fn reported_log_weight_equals_sum_of_hops() {
        testkit::reported_log_weight_equals_sum_of_hops(Algorithm::BellmanFord);
    }

    #[test]
    fn trace_is_nonempty_for_reachable_dst() {
        testkit::trace_is_nonempty_for_reachable_dst(Algorithm::BellmanFord);
    }

    #[test]
    fn every_trace_event_references_real_nodes() {
        testkit::every_trace_event_references_real_nodes(Algorithm::BellmanFord);
    }

    #[test]
    fn prefers_cheaper_pool_on_same_pair() {
        testkit::prefers_cheaper_pool_on_same_pair(Algorithm::BellmanFord);
    }

    #[test]
    fn amount_out_matches_simulate_path() {
        testkit::amount_out_matches_simulate_path(Algorithm::BellmanFord);
    }

    #[test]
    fn product_of_rates_equals_exp_neg_total_log_weight() {
        testkit::product_of_rates_equals_exp_neg_total_log_weight(Algorithm::BellmanFord);
    }

    #[test]
    fn solve_on_generated_graph_returns_valid_path() {
        testkit::solve_on_generated_graph_returns_valid_path(Algorithm::BellmanFord);
    }

    // --- BF-specific properties.

    #[test]
    fn detects_injected_arb_and_verifies_exact_profit() {
        testkit::bf_detects_injected_arb_and_verifies_exact_profit();
    }

    #[test]
    fn cycle_product_of_rates_greater_than_one() {
        testkit::bf_cycle_product_of_rates_greater_than_one();
    }

    #[test]
    fn arb_free_zero_fee_graph_returns_no_cycle() {
        testkit::bf_arb_free_zero_fee_graph_returns_no_cycle();
    }

    #[test]
    fn cycle_is_a_closed_loop_of_real_pools() {
        testkit::bf_cycle_is_a_closed_loop_of_real_pools();
    }

    #[test]
    fn cycle_output_matches_manual_simulate_path_replay() {
        testkit::bf_cycle_output_matches_manual_simulate_path_replay();
    }
}
