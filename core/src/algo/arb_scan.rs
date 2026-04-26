use std::collections::HashMap;

use alloy_primitives::{Address, U256};

use crate::algo::gas::GasModel;
use crate::algo::{Outcome, SolveResult};
use crate::graph::Graph;
use crate::trace::Step;

// Targeted arbitrage scan: find the most profitable simple cycle that
// starts and ends in a user-chosen token. DFS-enumerate cycles up to
// MAX_HOPS, ternary-search the trade size on each, return the best.
//
// Different from BF's cycle detection: BF scans for any negative-log-
// weight cycle anywhere in the graph. Here we fix the entry token and
// rank by exact U256 profit at the optimal trade size, so cycles that
// look good marginally but lose to slippage are rejected.

pub const MAX_HOPS: usize = 4;

// Profit(amount) is concave on a constant-product cycle, so ternary
// search converges in log2 steps. 60 is wei-tight on any realistic range.
const TERNARY_ITERS: usize = 60;

pub fn scan_from(graph: &Graph, entry: Address, gas: &GasModel) -> SolveResult {
    let entry_idx = match graph.index_of(entry) {
        Some(i) => i,
        None => {
            return SolveResult {
                outcome: Outcome::NoPath,
                trace: Vec::new(),
            };
        }
    };

    let n = graph.num_tokens();

    let unique_neighbors: Vec<Vec<usize>> = (0..n)
        .map(|u| {
            let mut seen = vec![false; n];
            let mut out = Vec::new();
            for edge in &graph.adj[u] {
                if !seen[edge.to] {
                    seen[edge.to] = true;
                    out.push(edge.to);
                }
            }
            out
        })
        .collect();

    let mut by_pair: HashMap<(usize, usize), Vec<usize>> =
        HashMap::with_capacity(graph.pools.len());
    for (i, pool) in graph.pools.iter().enumerate() {
        let a = match graph.index_of(pool.token_a) {
            Some(x) => x,
            None => continue,
        };
        let b = match graph.index_of(pool.token_b) {
            Some(x) => x,
            None => continue,
        };
        let key = if a < b { (a, b) } else { (b, a) };
        by_pair.entry(key).or_default().push(i);
    }

    let mut best_profit_f64 = 0.0f64;
    let mut best_amount_in = U256::ZERO;
    let mut best_out = U256::ZERO;
    let mut best_cycle_nodes: Option<Vec<usize>> = None;
    let mut best_cycle_pools: Option<Vec<usize>> = None;
    let mut best_log_weight = 0.0f64;
    let mut trace: Vec<Step> = vec![Step::Visit(entry_idx)];

    let mut visited = vec![false; n];
    let mut stack: Vec<usize> = vec![entry_idx];
    visited[entry_idx] = true;

    dfs_cycle(
        entry_idx,
        entry_idx,
        MAX_HOPS,
        &unique_neighbors,
        &mut stack,
        &mut visited,
        &mut |cycle_nodes: &[usize]| {
            // Pick best pool per hop by marginal rate (zero-input).
            let mut pools_idx: Vec<usize> = Vec::with_capacity(cycle_nodes.len());
            let mut log_weight = 0.0f64;
            let mut ok = true;
            for hop in 0..cycle_nodes.len() {
                let u = cycle_nodes[hop];
                let v = if hop + 1 < cycle_nodes.len() {
                    cycle_nodes[hop + 1]
                } else {
                    cycle_nodes[0]
                };
                let key = if u < v { (u, v) } else { (v, u) };
                let candidates = match by_pair.get(&key) {
                    Some(c) => c,
                    None => {
                        ok = false;
                        break;
                    }
                };
                let from_addr = graph.tokens[u].address;
                let mut best_pool_idx: Option<usize> = None;
                let mut best_log_w = f64::INFINITY;
                for &pool_idx in candidates {
                    let lw = graph.pools[pool_idx].log_weight(from_addr);
                    if lw.is_finite() && lw < best_log_w {
                        best_log_w = lw;
                        best_pool_idx = Some(pool_idx);
                    }
                }
                match best_pool_idx {
                    Some(pool_idx) => {
                        pools_idx.push(pool_idx);
                        log_weight += best_log_w;
                    }
                    None => {
                        ok = false;
                        break;
                    }
                }
            }
            if !ok {
                return;
            }
            // Skip if marginal rate is already unprofitable.
            if log_weight >= 0.0 {
                return;
            }

            // Search ceiling: the entry-side reserve of the first pool.
            let first_pool = &graph.pools[pools_idx[0]];
            let entry_addr = graph.tokens[cycle_nodes[0]].address;
            let (r_in_first, _) = first_pool.reserves_for(entry_addr);
            if r_in_first.is_zero() {
                return;
            }

            let cycle_output = |amount: U256| -> U256 {
                let mut amt = amount;
                for hop in 0..cycle_nodes.len() {
                    let u = cycle_nodes[hop];
                    let pool = &graph.pools[pools_idx[hop]];
                    let from_addr = graph.tokens[u].address;
                    amt = pool.output_amount(from_addr, amt);
                    if amt.is_zero() {
                        return U256::ZERO;
                    }
                }
                amt
            };

            // Per-cycle gas (constant across the ternary search).
            let entry_token = &graph.tokens[cycle_nodes[0]];
            let cycle_hops = pools_idx.len();
            let gas_units = gas.gas_units(1, cycle_hops);
            let gas_cost =
                gas.gas_to_dst_token(gas_units, entry_token.true_price_usd, entry_token.decimals);
            let gas_cost_f = gas_cost.to_string().parse::<f64>().unwrap_or(0.0);

            // f64 is enough for the relative ordering ternary search needs.
            let profit = |amount: U256| -> f64 {
                let out = cycle_output(amount);
                let o = out.to_string().parse::<f64>().unwrap_or(0.0);
                let i = amount.to_string().parse::<f64>().unwrap_or(0.0);
                o - i - gas_cost_f
            };

            // Ternary search on [1, r_in_first/2]; concave profit hump.
            let mut lo = U256::from(1u64);
            let mut hi = r_in_first / U256::from(2u64);
            if hi <= lo {
                return;
            }
            for _ in 0..TERNARY_ITERS {
                let span = hi - lo;
                let third = span / U256::from(3u64);
                if third.is_zero() {
                    break;
                }
                let m1 = lo + third;
                let m2 = hi - third;
                if profit(m1) < profit(m2) {
                    lo = m1;
                } else {
                    hi = m2;
                }
            }
            let optimal = (lo + hi) / U256::from(2u64);
            let optimal_out = cycle_output(optimal);
            if optimal_out <= optimal {
                return; // arb closed before reaching positive territory
            }
            let optimal_profit_f64 = profit(optimal);
            if optimal_profit_f64 <= 0.0 {
                return; // gas eats the profit
            }
            if optimal_profit_f64 <= best_profit_f64 {
                return;
            }

            best_profit_f64 = optimal_profit_f64;
            best_amount_in = optimal;
            best_out = optimal_out;
            best_cycle_nodes = Some(cycle_nodes.to_vec());
            best_cycle_pools = Some(pools_idx);
            best_log_weight = log_weight;
        },
    );

    // Trace only the winning cycle — DFS exhaustion would be O(V^MAX_HOPS)
    // events which chokes the JS playback for no animation value.
    if let (Some(cycle_nodes), Some(cycle_pools)) =
        (best_cycle_nodes.as_ref(), best_cycle_pools.as_ref())
    {
        let mut running = 0.0f64;
        for hop in 0..cycle_nodes.len() {
            let u = cycle_nodes[hop];
            let v = if hop + 1 < cycle_nodes.len() {
                cycle_nodes[hop + 1]
            } else {
                cycle_nodes[0]
            };
            let pool_idx = cycle_pools[hop];
            let from_addr = graph.tokens[u].address;
            running += graph.pools[pool_idx].log_weight(from_addr);
            trace.push(Step::Relax {
                from: u,
                to: v,
                new_distance: running,
            });
            trace.push(Step::Visit(v));
        }
    }

    match (best_cycle_nodes, best_cycle_pools) {
        (Some(cycle_nodes), Some(cycle_pools)) => {
            let cycle: Vec<Address> = cycle_nodes.iter().map(|&i| graph.address_of(i)).collect();
            let pools_used: Vec<Address> = cycle_pools
                .iter()
                .map(|&i| graph.pools[i].address)
                .collect();
            let product_of_rates = (-best_log_weight).exp();
            let entry_token_final = graph
                .tokens
                .iter()
                .find(|t| t.address == cycle[0])
                .expect("cycle[0] must be a graph token");
            let final_gas_units = gas.gas_units(1, pools_used.len());
            let final_gas_cost = gas.gas_to_dst_token(
                final_gas_units,
                entry_token_final.true_price_usd,
                entry_token_final.decimals,
            );
            SolveResult {
                outcome: Outcome::NegativeCycle {
                    cycle,
                    pools_used,
                    product_of_rates,
                    amount_in: best_amount_in,
                    cycle_output: best_out,
                    gas_cost: final_gas_cost,
                },
                trace,
            }
        }
        _ => SolveResult {
            outcome: Outcome::NoPath,
            trace,
        },
    }
}

// Emit every simple cycle src -> ... -> src up to `remaining` edges.
// Cycles come back as `[start, n1, n2, ..., nk]` — the close back to
// `start` is implicit.
fn dfs_cycle(
    current: usize,
    start: usize,
    remaining: usize,
    unique_neighbors: &[Vec<usize>],
    stack: &mut Vec<usize>,
    visited: &mut [bool],
    emit: &mut impl FnMut(&[usize]),
) {
    if remaining == 0 {
        return;
    }
    for &to in &unique_neighbors[current] {
        if to == start {
            // Need ≥ 2 intermediates — a 1-hop "cycle" is a round-trip
            // through one pool which can never arb.
            if stack.len() >= 2 {
                emit(stack);
            }
            continue;
        }
        if visited[to] {
            continue;
        }
        visited[to] = true;
        stack.push(to);
        dfs_cycle(
            to,
            start,
            remaining - 1,
            unique_neighbors,
            stack,
            visited,
            emit,
        );
        stack.pop();
        visited[to] = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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

    fn pool(pool_byte: u8, a: Address, b: Address, ra: u64, rb: u64, fee: u16) -> Pool {
        Pool {
            address: addr(pool_byte),
            token_a: a,
            token_b: b,
            reserve_a: U256::from(ra),
            reserve_b: U256::from(rb),
            fee_bps: fee,
            venue: "Test".into(),
        }
    }

    #[test]
    fn finds_profitable_triangle_through_user_token() {
        // Same A-B-C-A triangle used in bf_detects_injected_arb_and_verifies_exact_profit,
        // perturbed so the cycle through A is profitable.
        let a = addr(1);
        let b = addr(2);
        let c = addr(3);
        let g = Graph::new(
            vec![tok(1, "A"), tok(2, "B"), tok(3, "C")],
            vec![
                pool(0xA0, a, b, 1_000_000_000, 100_000_000_000, 0),
                pool(0xA1, b, c, 1_000_000_000, 100_000_000_000, 0),
                pool(0xA2, a, c, 1_100_000_000, 9_090_909_090_909, 0),
            ],
        );
        let r = scan_from(&g, a, &GasModel::off());
        match &r.outcome {
            Outcome::NegativeCycle {
                cycle,
                pools_used,
                product_of_rates,
                cycle_output,
                amount_in,
                ..
            } => {
                // Cycle must start at A.
                assert_eq!(cycle[0], a);
                assert_eq!(cycle.len(), 3);
                assert_eq!(pools_used.len(), 3);
                assert!(*product_of_rates > 1.0);
                assert!(*cycle_output > *amount_in);
            }
            other => panic!("expected NegativeCycle through A, got {other:?}"),
        }
    }

    #[test]
    fn returns_no_path_on_fair_graph() {
        let a = addr(1);
        let b = addr(2);
        let c = addr(3);
        // Exactly-consistent triangle: cycle product = 1 (no arb).
        let g = Graph::new(
            vec![tok(1, "A"), tok(2, "B"), tok(3, "C")],
            vec![
                pool(0xA0, a, b, 1_000_000_000, 100_000_000_000, 0),
                pool(0xA1, b, c, 1_000_000_000, 100_000_000_000, 0),
                pool(0xA2, a, c, 1_000_000_000, 10_000_000_000_000, 0),
            ],
        );
        let r = scan_from(&g, a, &GasModel::off());
        assert!(
            matches!(r.outcome, Outcome::NoPath),
            "expected NoPath on fair graph, got {:?}",
            r.outcome
        );
    }

    #[test]
    fn unknown_entry_token_returns_no_path() {
        let a = addr(1);
        let b = addr(2);
        let g = Graph::new(
            vec![tok(1, "A"), tok(2, "B")],
            vec![pool(0xA0, a, b, 1_000, 1_000, 0)],
        );
        let r = scan_from(&g, addr(99), &GasModel::off());
        assert!(matches!(r.outcome, Outcome::NoPath));
    }
}
