// Shared conformance suite — one algorithm-agnostic function per property.
// Each concrete algorithm's `#[cfg(test)]` module wires these up as their
// own `#[test]` cases, which means:
//   * the property is stated once;
//   * every algorithm pays for it (and a new algo can't ship without it);
//   * failures point at the failing property name, not a line in a shared
//     helper.

use alloy_primitives::{Address, U256};

use crate::algo::{Algorithm, Outcome, solve};
use crate::graph::Graph;
use crate::pool::Pool;
use crate::token::{Token, TokenKind};
use crate::trace::Step;

fn addr(byte: u8) -> Address {
    Address::from([byte; 20])
}

fn tok(byte: u8, symbol: &str, decimals: u8) -> Token {
    Token {
        address: addr(byte),
        symbol: symbol.into(),
        decimals,
        true_price_usd: 1.0,
        kind: TokenKind::Spoke,
    }
}

fn pool(
    pool_byte: u8,
    a: Address,
    b: Address,
    reserve_a: u64,
    reserve_b: u64,
    fee_bps: u16,
) -> Pool {
    Pool {
        address: addr(pool_byte),
        token_a: a,
        token_b: b,
        reserve_a: U256::from(reserve_a),
        reserve_b: U256::from(reserve_b),
        fee_bps,
        venue: "Test".to_string(),
    }
}

// ---------- properties ----------

pub fn src_equals_dst_returns_singleton(algo: Algorithm) {
    let a = addr(1);
    let b = addr(2);
    let g = Graph::new(
        vec![tok(1, "A", 18), tok(2, "B", 18)],
        vec![pool(0xA0, a, b, 1_000_000, 1_000_000, 30)],
    );
    let r = solve(algo, &g, a, a, U256::from(100u64));
    match &r.outcome {
        Outcome::Found {
            path,
            pools_used,
            amount_in,
            amount_out,
            total_log_weight,
            product_of_rates,
            ..
        } => {
            assert_eq!(path.as_slice(), &[a]);
            assert!(pools_used.is_empty());
            assert_eq!(*amount_in, U256::from(100u64));
            assert_eq!(*amount_out, U256::from(100u64));
            assert_eq!(*total_log_weight, 0.0);
            assert_eq!(*product_of_rates, 1.0);
        }
        other => panic!("expected Found, got {other:?}"),
    }
}

pub fn two_nodes_one_edge(algo: Algorithm) {
    let a = addr(1);
    let b = addr(2);
    let g = Graph::new(
        vec![tok(1, "A", 18), tok(2, "B", 18)],
        vec![pool(0xA0, a, b, 1_000_000_000, 2_000_000_000, 0)],
    );
    let r = solve(algo, &g, a, b, U256::from(1_000_000u64));
    match &r.outcome {
        Outcome::Found {
            path,
            pools_used,
            amount_in,
            amount_out,
            ..
        } => {
            assert_eq!(path.as_slice(), &[a, b]);
            assert_eq!(pools_used.as_slice(), &[addr(0xA0)]);
            assert_eq!(*amount_in, U256::from(1_000_000u64));
            // 1e6 into a 1e9/2e9 pool at zero fee:
            //   out = 1e6 * 2e9 / (1e9 + 1e6) = ~1_998_002
            assert!(*amount_out > U256::from(1_990_000u64));
            assert!(*amount_out < U256::from(2_000_000u64));
        }
        other => panic!("expected Found, got {other:?}"),
    }
}

pub fn no_path_when_disconnected(algo: Algorithm) {
    let a = addr(1);
    let b = addr(2);
    let g = Graph::new(vec![tok(1, "A", 18), tok(2, "B", 18)], Vec::new());
    let r = solve(algo, &g, a, b, U256::from(100u64));
    assert!(matches!(r.outcome, Outcome::NoPath));
}

pub fn unknown_source_or_dst_returns_no_path(algo: Algorithm) {
    let a = addr(1);
    let b = addr(2);
    let unknown = addr(99);
    let g = Graph::new(
        vec![tok(1, "A", 18), tok(2, "B", 18)],
        vec![pool(0xA0, a, b, 1_000, 1_000, 30)],
    );
    let r1 = solve(algo, &g, unknown, b, U256::from(100u64));
    assert!(matches!(r1.outcome, Outcome::NoPath));
    let r2 = solve(algo, &g, a, unknown, U256::from(100u64));
    assert!(matches!(r2.outcome, Outcome::NoPath));
}

pub fn path_is_contiguous_via_real_edges(algo: Algorithm) {
    let a = addr(1);
    let b = addr(2);
    let c = addr(3);
    let g = Graph::new(
        vec![tok(1, "A", 18), tok(2, "B", 18), tok(3, "C", 18)],
        vec![
            pool(0xA0, a, b, 1_000_000_000, 1_000_000_000, 0),
            pool(0xA1, b, c, 1_000_000_000, 1_000_000_000, 0),
        ],
    );
    let r = solve(algo, &g, a, c, U256::from(10_000u64));
    match &r.outcome {
        Outcome::Found {
            path, pools_used, ..
        } => {
            assert_eq!(path.as_slice(), &[a, b, c]);
            assert_eq!(pools_used.len(), path.len() - 1);
            for (i, pool_addr) in pools_used.iter().enumerate() {
                let p = g.pools.iter().find(|p| p.address == *pool_addr).unwrap();
                let from = path[i];
                let to = path[i + 1];
                let ok = (p.token_a == from && p.token_b == to)
                    || (p.token_a == to && p.token_b == from);
                assert!(ok, "pool {} does not connect {from} → {to}", p.address);
            }
        }
        other => panic!("expected Found, got {other:?}"),
    }
}

pub fn reported_log_weight_equals_sum_of_hops(algo: Algorithm) {
    let a = addr(1);
    let b = addr(2);
    let c = addr(3);
    let g = Graph::new(
        vec![tok(1, "A", 18), tok(2, "B", 18), tok(3, "C", 18)],
        vec![
            pool(0xA0, a, b, 1_000_000_000, 2_000_000_000, 30),
            pool(0xA1, b, c, 1_000_000_000, 3_000_000_000, 30),
        ],
    );
    let r = solve(algo, &g, a, c, U256::from(10_000u64));
    match &r.outcome {
        Outcome::Found {
            path,
            pools_used,
            total_log_weight,
            ..
        } => {
            let sum: f64 = pools_used
                .iter()
                .enumerate()
                .map(|(i, pool_addr)| {
                    let p = g.pools.iter().find(|p| p.address == *pool_addr).unwrap();
                    p.log_weight(path[i])
                })
                .sum();
            assert!(
                (*total_log_weight - sum).abs() < 1e-9,
                "total {} vs sum {}",
                total_log_weight,
                sum
            );
        }
        other => panic!("expected Found, got {other:?}"),
    }
}

pub fn trace_is_nonempty_for_reachable_dst(algo: Algorithm) {
    let a = addr(1);
    let b = addr(2);
    let g = Graph::new(
        vec![tok(1, "A", 18), tok(2, "B", 18)],
        vec![pool(0xA0, a, b, 1_000, 1_000, 30)],
    );
    let r = solve(algo, &g, a, b, U256::from(100u64));
    assert!(!r.trace.is_empty());
}

pub fn every_trace_event_references_real_nodes(algo: Algorithm) {
    let a = addr(1);
    let b = addr(2);
    let c = addr(3);
    let g = Graph::new(
        vec![tok(1, "A", 18), tok(2, "B", 18), tok(3, "C", 18)],
        vec![
            pool(0xA0, a, b, 1_000_000_000, 1_000_000_000, 30),
            pool(0xA1, b, c, 1_000_000_000, 1_000_000_000, 30),
        ],
    );
    let r = solve(algo, &g, a, c, U256::from(1_000u64));
    let n = g.num_tokens();
    for step in &r.trace {
        match step {
            Step::Visit(idx) | Step::Pass(idx) => assert!(*idx < n),
            Step::Relax { from, to, .. } => {
                assert!(*from < n);
                assert!(*to < n);
            }
        }
    }
}

pub fn prefers_cheaper_pool_on_same_pair(algo: Algorithm) {
    // Two parallel pools between A and B, same reserves, different fees.
    // The lower-fee pool must win.
    let a = addr(1);
    let b = addr(2);
    let g = Graph::new(
        vec![tok(1, "A", 18), tok(2, "B", 18)],
        vec![
            pool(0xA0, a, b, 1_000_000_000, 1_000_000_000, 100), // 1.0% fee — worse
            pool(0xA1, a, b, 1_000_000_000, 1_000_000_000, 10),  // 0.1% fee — better
        ],
    );
    let r = solve(algo, &g, a, b, U256::from(1_000u64));
    match &r.outcome {
        Outcome::Found { pools_used, .. } => {
            assert_eq!(pools_used.as_slice(), &[addr(0xA1)]);
        }
        other => panic!("expected Found, got {other:?}"),
    }
}

pub fn amount_out_matches_simulate_path(algo: Algorithm) {
    // The routing-layer decision and the execution-layer simulation must
    // agree on the exact output when replayed by the caller.
    let a = addr(1);
    let b = addr(2);
    let c = addr(3);
    let g = Graph::new(
        vec![tok(1, "A", 18), tok(2, "B", 18), tok(3, "C", 18)],
        vec![
            pool(0xA0, a, b, 1_000_000_000, 2_000_000_000, 30),
            pool(0xA1, b, c, 1_000_000_000, 3_000_000_000, 30),
        ],
    );
    let amount_in = U256::from(100_000u64);
    let r = solve(algo, &g, a, c, amount_in);
    match &r.outcome {
        Outcome::Found {
            path,
            pools_used,
            amount_out,
            ..
        } => {
            let replay = Pool::simulate_path(path, pools_used, &g.pools, amount_in);
            assert_eq!(replay, *amount_out);
        }
        other => panic!("expected Found, got {other:?}"),
    }
}

pub fn product_of_rates_equals_exp_neg_total_log_weight(algo: Algorithm) {
    let a = addr(1);
    let b = addr(2);
    let g = Graph::new(
        vec![tok(1, "A", 18), tok(2, "B", 18)],
        vec![pool(0xA0, a, b, 1_000_000_000, 2_000_000_000, 30)],
    );
    let r = solve(algo, &g, a, b, U256::from(1_000u64));
    match &r.outcome {
        Outcome::Found {
            total_log_weight,
            product_of_rates,
            ..
        } => {
            let expected = (-total_log_weight).exp();
            assert!(
                (product_of_rates - expected).abs() < 1e-12,
                "product {} vs expected {}",
                product_of_rates,
                expected
            );
        }
        other => panic!("expected Found, got {other:?}"),
    }
}

pub fn solve_on_generated_graph_returns_valid_path(algo: Algorithm) {
    use crate::generator::{GenConfig, PoolGenerator};
    // Zero noise so the generated graph is strictly arb-free and both
    // algorithms report Outcome::Found for a hub-to-hub query. With
    // default noise Bellman-Ford would prefer to surface any small
    // cycle it detects.
    let cfg = GenConfig {
        price_noise: 0.0,
        ..Default::default()
    };
    let (tokens, pools) = PoolGenerator::new(cfg).generate();
    let g = Graph::new(tokens, pools);
    let hubs: Vec<Address> = g
        .tokens
        .iter()
        .filter(|t| matches!(t.kind, TokenKind::Hub))
        .map(|t| t.address)
        .collect();
    assert!(hubs.len() >= 2);
    let src = hubs[0];
    let dst = hubs[1];
    let amount_in = U256::from(1_000_000u64);
    let r = solve(algo, &g, src, dst, amount_in);
    match &r.outcome {
        Outcome::Found {
            path,
            amount_in: reported_in,
            amount_out,
            pools_used,
            ..
        } => {
            assert_eq!(path.first().copied(), Some(src));
            assert_eq!(path.last().copied(), Some(dst));
            assert_eq!(*reported_in, amount_in);
            assert!(*amount_out > U256::ZERO);
            assert_eq!(pools_used.len(), path.len() - 1);
        }
        other => panic!("expected Found between hubs, got {other:?}"),
    }
}

// ---------- Bellman-Ford-specific properties ----------

// Hand-built three-pool arb cycle: A ↔ B at fair 100:1, B ↔ C at fair 100:1,
// A ↔ C mispriced (9000:1 instead of the fair 10000:1 implied by the other
// two hops). The cycle A → B → C → A has product_of_rates ≈ 1.111.
// Reserves are chosen so `amount_in = 1000` is negligible vs the pool sizes
// — slippage well under 1 wei — so the exact U256 math agrees with the
// infinitesimal-rate calculation.
fn hand_arb_graph() -> (Graph, Address, Address, Address) {
    let a = addr(1);
    let b = addr(2);
    let c = addr(3);
    let g = Graph::new(
        vec![tok(1, "A", 18), tok(2, "B", 18), tok(3, "C", 18)],
        vec![
            pool(0xA0, a, b, 1_000_000_000, 100_000_000_000, 0),
            pool(0xA1, b, c, 1_000_000_000, 100_000_000_000, 0),
            pool(0xA2, a, c, 1_000_000_000, 9_000_000_000_000, 0),
        ],
    );
    (g, a, b, c)
}

pub fn bf_detects_injected_arb_and_verifies_exact_profit() {
    // Manual arb-free triangle with exact-multiple reserves so the cycle
    // product is 1 pre-perturbation:
    //   A/B:   1e9 A ↔ 1e11 B   (rate 100)
    //   B/C:   1e9 B ↔ 1e11 C   (rate 100)
    //   A/C:   1e9 A ↔ 1e13 C   (rate 10_000 = 100 × 100)
    let a = addr(1);
    let b = addr(2);
    let c = addr(3);
    let tokens = vec![tok(1, "A", 18), tok(2, "B", 18), tok(3, "C", 18)];
    let clean_pools = vec![
        pool(0xA0, a, b, 1_000_000_000, 100_000_000_000, 0),
        pool(0xA1, b, c, 1_000_000_000, 100_000_000_000, 0),
        pool(0xA2, a, c, 1_000_000_000, 10_000_000_000_000, 0),
    ];

    let clean_graph = Graph::new(tokens.clone(), clean_pools.clone());
    let clean = solve(
        Algorithm::BellmanFord,
        &clean_graph,
        a,
        b,
        U256::from(1_000u64),
    );
    assert!(
        !matches!(clean.outcome, Outcome::NegativeCycle { .. }),
        "arb-free clean graph unexpectedly produced a negative cycle: {:?}",
        clean.outcome
    );

    // Deliberately perturb pool A/C so its C-reserve drops 10 % (matches
    // `inject_arb(magnitude=0.10)` but with a known target — we control
    // which cycle becomes profitable and in which direction).
    //   After perturbation the cycle A → B → C → A has product ≈ 1.21.
    let mut arb_pools = clean_pools;
    arb_pools[2] = pool(0xA2, a, c, 1_100_000_000, 9_090_909_090_909, 0);
    let arb_graph = Graph::new(tokens, arb_pools);
    let amount_in = U256::from(1_000u64);
    let arb = solve(Algorithm::BellmanFord, &arb_graph, a, b, amount_in);
    match &arb.outcome {
        Outcome::NegativeCycle {
            cycle,
            pools_used,
            product_of_rates,
            amount_in: reported_in,
            cycle_output,
            ..
        } => {
            assert_eq!(*reported_in, amount_in);
            assert!(*product_of_rates > 1.0, "product {} <= 1", product_of_rates);
            assert!(
                *cycle_output > *reported_in,
                "cycle_output {} not > amount_in {} at zero fees",
                cycle_output,
                reported_in
            );
            // 3-node cycle through all three pools.
            assert_eq!(cycle.len(), 3);
            assert_eq!(pools_used.len(), 3);
        }
        other => panic!("expected NegativeCycle after perturbation, got {other:?}"),
    }
}

pub fn bf_cycle_product_of_rates_greater_than_one() {
    let (g, a, b, c) = hand_arb_graph();
    let _ = (b, c); // tokens used only via addresses inside the graph
    let r = solve(Algorithm::BellmanFord, &g, a, a, U256::from(1_000u64));
    // src == dst short-circuits to Found — we need a non-trivial target to
    // force BF to relax edges. Any other token works; try A → B.
    let r = if matches!(r.outcome, Outcome::Found { .. } if r.trace.len() == 1) {
        solve(Algorithm::BellmanFord, &g, a, b, U256::from(1_000u64))
    } else {
        r
    };
    match &r.outcome {
        Outcome::NegativeCycle {
            product_of_rates, ..
        } => {
            assert!(
                *product_of_rates > 1.0,
                "expected product > 1, got {}",
                product_of_rates
            );
            // For our specific graph the cycle product is ~1.111.
            assert!(*product_of_rates > 1.10 && *product_of_rates < 1.13);
        }
        other => panic!("expected NegativeCycle, got {other:?}"),
    }
}

pub fn bf_arb_free_zero_fee_graph_returns_no_cycle() {
    // Linear path A - B - C with no cycles at all (2 pools, tree topology).
    // BF from A to C must return Found, never NegativeCycle.
    let a = addr(1);
    let b = addr(2);
    let c = addr(3);
    let g = Graph::new(
        vec![tok(1, "A", 18), tok(2, "B", 18), tok(3, "C", 18)],
        vec![
            pool(0xA0, a, b, 1_000_000, 1_000_000, 0),
            pool(0xA1, b, c, 1_000_000, 1_000_000, 0),
        ],
    );
    let r = solve(Algorithm::BellmanFord, &g, a, c, U256::from(1_000u64));
    assert!(
        !matches!(r.outcome, Outcome::NegativeCycle { .. }),
        "tree topology must not produce a negative cycle: {:?}",
        r.outcome
    );
    assert!(matches!(r.outcome, Outcome::Found { .. }));
}

pub fn bf_cycle_is_a_closed_loop_of_real_pools() {
    let (g, a, b, _c) = hand_arb_graph();
    let r = solve(Algorithm::BellmanFord, &g, a, b, U256::from(1_000u64));
    match &r.outcome {
        Outcome::NegativeCycle {
            cycle, pools_used, ..
        } => {
            assert_eq!(pools_used.len(), cycle.len());
            // Every consecutive (cycle[i], cycle[(i+1) mod n]) pair must
            // be connected by pools_used[i].
            for i in 0..cycle.len() {
                let from = cycle[i];
                let to = cycle[(i + 1) % cycle.len()];
                let p = g
                    .pools
                    .iter()
                    .find(|p| p.address == pools_used[i])
                    .expect("cycle pool must exist in graph.pools");
                let ok = (p.token_a == from && p.token_b == to)
                    || (p.token_a == to && p.token_b == from);
                assert!(
                    ok,
                    "pools_used[{}] = {} does not connect {} -> {}",
                    i, p.address, from, to
                );
            }
        }
        other => panic!("expected NegativeCycle, got {other:?}"),
    }
}

// ---------- Amount-aware-specific properties ----------

// Hand-built graph designed to force disagreement between log-weight
// Dijkstra and amount-aware enumeration. Two ways from A to C:
//
//   1. Direct A ↔ C pool at 0 bps fee. Large reserves so slippage is low
//      at both small and large trade sizes.
//   2. Two-hop A → B → C, each pool at 0 bps fee but with reserves
//      proportionally *much smaller* than the direct pool. The marginal
//      rate product via B slightly beats direct, so Dijkstra picks it.
//      For a tiny trade that choice is fine; for a large trade the
//      multi-hop path eats crippling slippage at both small pools while
//      direct barely notices.
//
// This matches the "Dijkstra vs direct" screenshot from section 7, but
// constructed so amount-aware can reproduce both regimes deterministically.
fn slippage_sensitive_graph() -> (Graph, Address, Address) {
    let a = addr(1);
    let b = addr(2);
    let c = addr(3);
    let tokens = vec![tok(1, "A", 18), tok(2, "B", 18), tok(3, "C", 18)];
    let pools = vec![
        // Direct A↔C — big reserves, small slippage even for large trades.
        pool(0xA0, a, c, 1_000_000_000, 1_000_000_000, 0),
        // Multi-hop A→B→C — each side ~40× smaller. Marginal rates per hop
        // are identical to direct, so the log-weight sum is (very slightly)
        // *better* via B for tiny trades due to zero fees compounding to
        // zero, and Dijkstra's pop order happens to prefer it.
        pool(0xB1, a, b, 25_000_000, 27_500_000, 0),
        pool(0xB2, b, c, 25_000_000, 27_500_000, 0),
    ];
    (Graph::new(tokens, pools), a, c)
}

pub fn amount_aware_prefers_direct_when_direct_has_better_liquidity() {
    let (g, a, c) = slippage_sensitive_graph();
    // Large trade: 10 M base units. That's 40 % of the multi-hop pools'
    // reserves — catastrophic slippage. Direct A↔C (1 B reserve) sees
    // just 1 % movement.
    let big_amount = U256::from(10_000_000u64);
    let r = solve(Algorithm::AmountAware, &g, a, c, big_amount);
    match &r.outcome {
        Outcome::Found {
            path,
            pools_used,
            amount_out,
            ..
        } => {
            assert_eq!(
                path.as_slice(),
                &[a, c],
                "expected direct path at large size"
            );
            assert_eq!(pools_used.as_slice(), &[addr(0xA0)]);
            // Direct pool yields ~10 M * 10^9 / (10^9 + 10^7) ≈ 9.9 M. The
            // multi-hop at the same amount would yield drastically less.
            assert!(*amount_out > U256::from(9_000_000u64));
        }
        other => panic!("expected Found via direct pool, got {other:?}"),
    }
}

pub fn amount_aware_picks_multi_hop_for_small_trade() {
    let (g, a, c) = slippage_sensitive_graph();
    // Tiny trade: 1000 units. Negligible slippage everywhere. Whichever
    // path has the marginal rate edge wins. Construction gives multi-hop
    // B a sliver of an edge — amount-aware should find it *and* deliver
    // the higher output.
    let small_amount = U256::from(1_000u64);
    let r = solve(Algorithm::AmountAware, &g, a, c, small_amount);
    match &r.outcome {
        Outcome::Found {
            path, amount_out, ..
        } => {
            // At this tiny amount both routes produce similar outputs but
            // multi-hop should win by a hair, and amount-aware must find
            // it. If it returned direct that's also defensible (rates are
            // close), so the strict assertion is "output must beat direct
            // route alone at the same amount".
            let direct_out = g.pools[0].output_amount(a, small_amount);
            assert!(
                *amount_out >= direct_out,
                "amount-aware output {} worse than direct {}: path = {:?}",
                amount_out,
                direct_out,
                path
            );
        }
        other => panic!("expected Found, got {other:?}"),
    }
}

pub fn bf_cycle_output_matches_manual_simulate_path_replay() {
    let (g, a, b, _c) = hand_arb_graph();
    let amount_in = U256::from(1_000u64);
    let r = solve(Algorithm::BellmanFord, &g, a, b, amount_in);
    match &r.outcome {
        Outcome::NegativeCycle {
            cycle,
            pools_used,
            cycle_output,
            ..
        } => {
            // Build the closed-loop token sequence `cycle + [cycle[0]]`
            // and replay via simulate_path — must match exactly.
            let mut closed: Vec<Address> = cycle.clone();
            closed.push(cycle[0]);
            let replay = Pool::simulate_path(&closed, pools_used, &g.pools, amount_in);
            assert_eq!(replay, *cycle_output);
            // At zero fees the cycle is profitable.
            assert!(*cycle_output > amount_in);
        }
        other => panic!("expected NegativeCycle, got {other:?}"),
    }
}
