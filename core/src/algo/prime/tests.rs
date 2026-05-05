use std::collections::HashSet;

use alloy_primitives::{Address, U256};

use super::*;
use crate::algo::{Algorithm, solve};
use crate::pool::Pool;
use crate::token::{Token, TokenKind};

// === fixtures ==========================================================

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

// === core invariants (carried over from V1) ============================

#[test]
fn src_equals_dst_returns_singleton_leg() {
    let a = addr(1);
    let g = Graph::new(vec![tok(1, "A")], Vec::new());
    let r = solve(Algorithm::Prime, &g, a, a, U256::from(100u64));
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
    let r = solve(Algorithm::Prime, &g, a, b, U256::from(100u64));
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
    let r = solve(Algorithm::Prime, &g, a, b, U256::from(100u64));
    match r.outcome {
        Outcome::FoundSplit { legs, .. } => {
            assert_eq!(legs.len(), 1);
            assert_eq!(legs[0].path, vec![a, b]);
        }
        other => panic!("expected FoundSplit, got {other:?}"),
    }
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
    let r = solve(Algorithm::Prime, &g, a, c, amount);
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
fn pool_disjoint_paths_match_fw() {
    // A─┬─B─D  (two independent routes — no shared pool)
    //   └─C─D
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

    let prime = solve(Algorithm::Prime, &g, a, d, amount);
    let fw = solve(Algorithm::SplitFw, &g, a, d, amount);

    let prime_out = match prime.outcome {
        Outcome::FoundSplit { amount_out, .. } => amount_out,
        other => panic!("expected FoundSplit, got {other:?}"),
    };
    let fw_out = match fw.outcome {
        Outcome::FoundSplit { amount_out, .. } => amount_out,
        other => panic!("expected FoundSplit, got {other:?}"),
    };

    let delta = if prime_out > fw_out {
        prime_out - fw_out
    } else {
        fw_out - prime_out
    };
    assert!(
        delta * U256::from(100u64) <= fw_out.max(prime_out),
        "PRIME {prime_out} vs FW {fw_out} differ by more than 1%"
    );
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
        Algorithm::Prime,
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

// === spec-checklist additions ==========================================

#[test]
fn hub_set_picks_top_liquidity() {
    let a = addr(1);
    let b = addr(2);
    let c = addr(3);
    let g = Graph::new(
        vec![tok(1, "A"), tok(2, "B"), tok(3, "C")],
        vec![
            pool(0xA1, a, b, 10_000_000, 10_000_000),
            pool(0xA2, b, c, 1_000, 1_000),
        ],
    );
    let hs = build_hub_set(&g, 2);
    let b_idx = g.index_of(b).unwrap();
    let a_idx = g.index_of(a).unwrap();
    assert!(hs.hubs.contains(&b_idx));
    assert!(hs.hubs.contains(&a_idx));
    assert_eq!(hs.rank.get(&b_idx).copied(), Some(0));
}

#[test]
fn merge_and_expand_groups_by_token_sequence() {
    // Two paths share token sequence A→B→C but use different pools at hop 0.
    // Merging should produce one MultiEdgePath whose pools_at_hop[0] holds
    // both alternatives.
    let a = addr(1);
    let b = addr(2);
    let c = addr(3);
    let g = Graph::new(
        vec![tok(1, "A"), tok(2, "B"), tok(3, "C")],
        vec![
            pool(0xA1, a, b, 1_000_000, 1_000_000),
            pool(0xA2, a, b, 2_000_000, 2_000_000),
            pool(0xA3, b, c, 1_000_000, 1_000_000),
        ],
    );
    let p1 = MultiEdgePath::from_single(vec![0, 1, 2], vec![0, 2], 0.5);
    let p2 = MultiEdgePath::from_single(vec![0, 1, 2], vec![1, 2], 0.5);
    let merged = merge_and_expand(vec![(p1, 0.5), (p2, 0.5)], &g);
    assert_eq!(merged.len(), 1);
    assert_eq!(merged[0].tokens, vec![0, 1, 2]);
    assert_eq!(merged[0].pools_at_hop[0].len(), 2);
    assert_eq!(merged[0].pools_at_hop[1].len(), 1);
}

#[test]
fn parallel_fee_tier_split_is_used() {
    // Two parallel pools between A and B, identical reserves. With a large
    // trade, splitting should beat single-pool routing.
    let a = addr(1);
    let b = addr(2);
    let g = Graph::new(
        vec![tok(1, "A"), tok(2, "B")],
        vec![
            pool(0xA1, a, b, 1_000_000, 1_000_000),
            pool(0xA2, a, b, 1_000_000, 1_000_000),
        ],
    );
    let amount = U256::from(500_000u64);
    let r = solve(Algorithm::Prime, &g, a, b, amount);
    let out = match r.outcome {
        Outcome::FoundSplit { amount_out, .. } => amount_out,
        other => panic!("expected FoundSplit, got {other:?}"),
    };
    // Single-pool 500k → ~332k. Two-pool 250k each → ~398k total.
    assert!(
        out > U256::from(332_000u64),
        "split routing should exceed single-pool"
    );
}

#[test]
fn tau_is_non_decreasing_across_path_admissions() {
    // In our convention τ = max marginal rate (∂out/∂in) at equilibrium.
    // Admitting a path with mp(0) > τ either keeps τ or raises it as flow
    // redistributes to higher-rate routes. (Paper's convention has τ as a
    // cost/price and is non-increasing — same invariant under inversion.)
    let a = addr(1);
    let b = addr(2);
    let c = addr(3);
    let d = addr(4);
    let e = addr(5);
    let g = Graph::new(
        vec![
            tok(1, "A"),
            tok(2, "B"),
            tok(3, "C"),
            tok(4, "D"),
            tok(5, "E"),
        ],
        vec![
            pool(0x01, a, b, 1_000_000, 1_000_000),
            pool(0x02, a, c, 1_000_000, 1_000_000),
            pool(0x03, c, b, 1_000_000, 1_000_000),
            pool(0x04, a, d, 1_000_000, 1_000_000),
            pool(0x05, d, b, 1_000_000, 1_000_000),
            pool(0x06, a, e, 1_000_000, 1_000_000),
            pool(0x07, e, b, 1_000_000, 1_000_000),
        ],
    );

    let cfg = PrimeConfig::default();
    let hubs = build_hub_set(&g, 30);
    let shortcuts = build_shortcut_index(&g, &hubs, &cfg);
    let core = CoreGraphView {
        graph: &g,
        hubs: &hubs,
        s: 0,
        t: 1,
    };
    let amount = U256::from(100_000u64);

    let mut paths_v: Vec<MultiEdgePath> = Vec::new();
    let mut used = HashSet::new();
    let mut taus: Vec<f64> = Vec::new();

    let direct = DiscoveredPath {
        tokens: vec![0, 1],
        pools: vec![0],
    };
    for &p in &direct.pools {
        used.insert(p);
    }
    paths_v.push(MultiEdgePath::from_single(direct.tokens, direct.pools, 1.0));

    let mut tau = super::asgm::run_asgm(&mut paths_v, &g, amount, &cfg.asgm);
    taus.push(tau);

    for _ in 0..cfg.max_paths {
        let Some(disc) = find_path(
            &g,
            &core,
            &shortcuts,
            0,
            1,
            amount,
            tau,
            &used,
            cfg.max_hops,
        ) else {
            break;
        };
        for &p in &disc.pools {
            used.insert(p);
        }
        paths_v.push(MultiEdgePath::from_single(disc.tokens, disc.pools, 0.0));
        tau = super::asgm::run_asgm(&mut paths_v, &g, amount, &cfg.asgm);
        taus.push(tau);
    }

    for w in taus.windows(2) {
        assert!(w[1] >= w[0] - 1e-6, "τ decreased: {} → {}", w[0], w[1]);
    }
}
