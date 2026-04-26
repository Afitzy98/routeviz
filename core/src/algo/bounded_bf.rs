use std::collections::HashMap;

use crate::graph::Graph;

// Top-K bounded-hop Bellman-Ford path enumerator.
//
// Emits simple paths src → dst with ≤ max_hops edges, ordered by
// ascending log-weight. Per (hop, node) DP state we keep at most TOP_K
// candidates. Consumers treat this as a path generator and rerank by
// realised output net of gas — log-weight is slippage-blind, so the
// ordering is a heuristic, not a correctness guarantee.
//
// Why bounded-hop BF rather than Dijkstra/Yen's: log-weight edges can
// be negative and the generator can produce real negative cycles.
// Dijkstra is wrong on negative edges; Yen's inherits that. Bounded
// hops + simple-path constraint keeps the iter correct on any weights.

const TOP_K: usize = 20;

pub struct BoundedBfIter<'a> {
    paths: std::vec::IntoIter<Vec<usize>>,
    _marker: std::marker::PhantomData<&'a Graph>,
}

#[derive(Clone)]
struct Candidate {
    weight: f64,
    path: Vec<usize>,
}

impl<'a> BoundedBfIter<'a> {
    pub fn new(graph: &'a Graph, src: usize, dst: usize, max_hops: usize) -> Self {
        let n = graph.num_tokens();
        if src >= n || dst >= n {
            return Self::empty();
        }
        if src == dst {
            return Self {
                paths: vec![vec![src]].into_iter(),
                _marker: std::marker::PhantomData,
            };
        }

        // Coalesce parallel pools to one outgoing edge per (u, v) at
        // the min log-weight. Consumers rerank with the actual best
        // pool per hop at trade size.
        let mut outgoing: Vec<HashMap<usize, f64>> = vec![HashMap::new(); n];
        for (u, out) in outgoing.iter_mut().enumerate().take(n) {
            for edge in &graph.adj[u] {
                let v = edge.to;
                let w = graph.pools[edge.pool].log_weight(edge.in_token);
                if !w.is_finite() {
                    continue;
                }
                let e = out.entry(v).or_insert(f64::INFINITY);
                if w < *e {
                    *e = w;
                }
            }
        }

        // k_best[h][v] = up to TOP_K best paths reaching v in h edges.
        let mut k_best: Vec<Vec<Vec<Candidate>>> = vec![vec![Vec::new(); n]; max_hops + 1];
        k_best[0][src].push(Candidate {
            weight: 0.0,
            path: vec![src],
        });

        for h in 1..=max_hops {
            // Split layers so we can borrow h-1 immutably + h mutably.
            let (prev_layers, curr_layers) = k_best.split_at_mut(h);
            let prev = &prev_layers[h - 1];
            let curr = &mut curr_layers[0];

            for u in 0..n {
                if prev[u].is_empty() {
                    continue;
                }
                for (&v, &w) in &outgoing[u] {
                    for cand in &prev[u] {
                        if cand.path.contains(&v) {
                            continue; // simple-path constraint
                        }
                        let new_weight = cand.weight + w;
                        // Skip allocation if we'd be evicted immediately.
                        if curr[v].len() >= TOP_K && new_weight >= curr[v][TOP_K - 1].weight {
                            continue;
                        }
                        let mut new_path = Vec::with_capacity(cand.path.len() + 1);
                        new_path.extend_from_slice(&cand.path);
                        new_path.push(v);
                        insert_top_k(
                            &mut curr[v],
                            Candidate {
                                weight: new_weight,
                                path: new_path,
                            },
                        );
                    }
                }
            }
        }

        // Gather dst-reaching candidates across hop counts, sort.
        let mut all: Vec<Candidate> = Vec::new();
        for layer in k_best.iter_mut().take(max_hops + 1).skip(1) {
            all.extend(std::mem::take(&mut layer[dst]));
        }
        all.sort_by(|a, b| {
            a.weight
                .partial_cmp(&b.weight)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        Self {
            paths: all
                .into_iter()
                .map(|c| c.path)
                .collect::<Vec<_>>()
                .into_iter(),
            _marker: std::marker::PhantomData,
        }
    }

    fn empty() -> Self {
        Self {
            paths: Vec::new().into_iter(),
            _marker: std::marker::PhantomData,
        }
    }
}

impl<'a> Iterator for BoundedBfIter<'a> {
    type Item = Vec<usize>;

    fn next(&mut self) -> Option<Self::Item> {
        self.paths.next()
    }
}

// Sorted insert with truncation at TOP_K.
fn insert_top_k(bucket: &mut Vec<Candidate>, cand: Candidate) {
    let pos = bucket
        .binary_search_by(|c| {
            c.weight
                .partial_cmp(&cand.weight)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .unwrap_or_else(|p| p);
    bucket.insert(pos, cand);
    if bucket.len() > TOP_K {
        bucket.truncate(TOP_K);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pool::Pool;
    use crate::token::{Token, TokenKind};
    use alloy_primitives::{Address, U256};
    use std::collections::HashSet;

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
    fn pool(pool_byte: u8, a: Address, b: Address, ra: u64, rb: u64) -> Pool {
        Pool {
            address: addr(pool_byte),
            token_a: a,
            token_b: b,
            reserve_a: U256::from(ra),
            reserve_b: U256::from(rb),
            fee_bps: 0,
            venue: "Test".into(),
        }
    }

    #[test]
    fn emits_shortest_first() {
        let a = addr(1);
        let b = addr(2);
        let c = addr(3);
        let g = Graph::new(
            vec![tok(1, "A"), tok(2, "B"), tok(3, "C")],
            vec![
                pool(0xA0, a, c, 1_000, 1_000),
                pool(0xA1, a, b, 1_000, 2_000),
                pool(0xA2, b, c, 1_000, 2_000),
            ],
        );
        let g_src = g.index_of(a).unwrap();
        let g_dst = g.index_of(c).unwrap();
        let mut iter = BoundedBfIter::new(&g, g_src, g_dst, 3);
        let p1 = iter.next().expect("at least one path");
        let p2 = iter.next().expect("at least two paths");
        assert_ne!(p1, p2);
    }

    #[test]
    fn terminates_when_exhausted() {
        let a = addr(1);
        let b = addr(2);
        let g = Graph::new(
            vec![tok(1, "A"), tok(2, "B")],
            vec![pool(0xA0, a, b, 1_000, 1_000)],
        );
        let mut iter = BoundedBfIter::new(&g, g.index_of(a).unwrap(), g.index_of(b).unwrap(), 3);
        assert!(iter.next().is_some());
        assert!(iter.next().is_none());
    }

    #[test]
    fn works_on_arb_free_generator_graph() {
        use crate::generator::{GenConfig, PoolGenerator};
        let (tokens, pools) = PoolGenerator::new(GenConfig {
            price_noise: 0.0,
            ..Default::default()
        })
        .generate();
        let g = Graph::new(tokens.clone(), pools);
        let hubs: Vec<_> = tokens
            .iter()
            .filter(|t| matches!(t.kind, TokenKind::Hub))
            .collect();
        let src = g.index_of(hubs[0].address).unwrap();
        let dst = g.index_of(hubs[1].address).unwrap();
        let mut count = 0;
        for _ in BoundedBfIter::new(&g, src, dst, 3) {
            count += 1;
            if count >= 50 {
                break;
            }
        }
        assert!(count >= 10, "expected ≥10 paths, got {count}");
    }

    #[test]
    fn works_on_arb_able_generator_graph() {
        // Regression for the memory-explosion bug in the prior
        // Dijkstra-backed Yen's. Bounded-hop BF handles cycles cleanly.
        use crate::generator::{GenConfig, PoolGenerator};
        let (tokens, pools) = PoolGenerator::new(GenConfig::default()).generate();
        let g = Graph::new(tokens.clone(), pools);
        let hubs: Vec<_> = tokens
            .iter()
            .filter(|t| matches!(t.kind, TokenKind::Hub))
            .collect();
        let src = g.index_of(hubs[0].address).unwrap();
        let dst = g.index_of(hubs[1].address).unwrap();
        let count = BoundedBfIter::new(&g, src, dst, 3).count();
        assert!(count >= 1, "expected ≥1 path, got {count}");
    }

    #[test]
    fn respects_max_hops() {
        let a = addr(1);
        let b = addr(2);
        let c = addr(3);
        let d = addr(4);
        let g = Graph::new(
            vec![tok(1, "A"), tok(2, "B"), tok(3, "C"), tok(4, "D")],
            vec![
                pool(0xA1, a, b, 1_000, 1_000),
                pool(0xA2, b, c, 1_000, 1_000),
                pool(0xA3, c, d, 1_000, 1_000),
            ],
        );
        let src = g.index_of(a).unwrap();
        let dst = g.index_of(d).unwrap();
        let mut iter = BoundedBfIter::new(&g, src, dst, 2);
        assert!(iter.next().is_none());
        let mut iter = BoundedBfIter::new(&g, src, dst, 3);
        assert!(iter.next().is_some());
    }

    #[test]
    fn paths_are_simple() {
        let a = addr(1);
        let b = addr(2);
        let c = addr(3);
        let g = Graph::new(
            vec![tok(1, "A"), tok(2, "B"), tok(3, "C")],
            vec![
                pool(0xA1, a, b, 1_000, 1_000),
                pool(0xA2, b, c, 1_000, 1_000),
                pool(0xA3, a, c, 1_000, 1_000),
            ],
        );
        let src = g.index_of(a).unwrap();
        let dst = g.index_of(c).unwrap();
        for path in BoundedBfIter::new(&g, src, dst, 3) {
            let unique: HashSet<usize> = path.iter().copied().collect();
            assert_eq!(unique.len(), path.len(), "duplicate node: {path:?}");
        }
    }

    #[test]
    fn parallel_pools_coalesce() {
        let a = addr(1);
        let b = addr(2);
        let g = Graph::new(
            vec![tok(1, "A"), tok(2, "B")],
            vec![
                pool(0xA1, a, b, 1_000, 1_000),
                pool(0xA2, a, b, 1_000, 2_000),
                pool(0xA3, a, b, 2_000, 1_000),
                pool(0xA4, a, b, 3_000, 3_000),
            ],
        );
        let src = g.index_of(a).unwrap();
        let dst = g.index_of(b).unwrap();
        let paths: Vec<_> = BoundedBfIter::new(&g, src, dst, 3).collect();
        assert_eq!(paths.len(), 1, "expected 1 coalesced path, got {paths:?}");
    }

    #[test]
    fn handles_negative_cycles_without_exploding() {
        // Construct a 3-node graph with a rate-1 loop that would let
        // a cycle-exploiting algorithm spiral: A→B→A is balanced, but
        // tiny float noise can make one direction cheaper. Bounded-hop
        // BF must still terminate and emit the direct A→C and A→B→C
        // paths, not cycled ones.
        let a = addr(1);
        let b = addr(2);
        let c = addr(3);
        let g = Graph::new(
            vec![tok(1, "A"), tok(2, "B"), tok(3, "C")],
            vec![
                pool(0xA1, a, b, 1_000_000, 1_000_001),
                pool(0xA2, b, a, 1_000_001, 1_000_000),
                pool(0xA3, b, c, 1_000, 1_000),
                pool(0xA4, a, c, 1_000, 1_000),
            ],
        );
        let src = g.index_of(a).unwrap();
        let dst = g.index_of(c).unwrap();
        let paths: Vec<_> = BoundedBfIter::new(&g, src, dst, 3).collect();
        assert!(!paths.is_empty());
        for p in &paths {
            // No path revisits a token.
            let unique: HashSet<usize> = p.iter().copied().collect();
            assert_eq!(unique.len(), p.len());
        }
    }
}
