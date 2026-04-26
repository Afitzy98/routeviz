use std::collections::HashMap;

use alloy_primitives::Address;
use serde::{Deserialize, Serialize};

use crate::pool::Pool;
use crate::token::Token;

// Directed edge in dense usize index-space. `in_token` is carried so
// `Pool::output_amount` doesn't need to re-derive direction.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Edge {
    pub pool: usize,
    pub to: usize,
    pub in_token: Address,
}

// Directed token graph. Each Pool contributes two Edges (a→b and b→a).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Graph {
    pub tokens: Vec<Token>,
    pub pools: Vec<Pool>,
    pub token_index: HashMap<Address, usize>,
    pub adj: Vec<Vec<Edge>>,
}

impl Graph {
    pub fn new(tokens: Vec<Token>, pools: Vec<Pool>) -> Self {
        let token_index: HashMap<Address, usize> = tokens
            .iter()
            .enumerate()
            .map(|(i, t)| (t.address, i))
            .collect();
        assert_eq!(
            token_index.len(),
            tokens.len(),
            "Graph::new: duplicate token address in tokens vec"
        );

        let mut adj: Vec<Vec<Edge>> = vec![Vec::new(); tokens.len()];
        for (pool_idx, pool) in pools.iter().enumerate() {
            assert_ne!(
                pool.token_a, pool.token_b,
                "Graph::new: pool {} has identical tokens",
                pool.address
            );
            let a_idx = *token_index.get(&pool.token_a).unwrap_or_else(|| {
                panic!(
                    "Graph::new: pool {} references unknown token_a {}",
                    pool.address, pool.token_a
                )
            });
            let b_idx = *token_index.get(&pool.token_b).unwrap_or_else(|| {
                panic!(
                    "Graph::new: pool {} references unknown token_b {}",
                    pool.address, pool.token_b
                )
            });
            adj[a_idx].push(Edge {
                pool: pool_idx,
                to: b_idx,
                in_token: pool.token_a,
            });
            adj[b_idx].push(Edge {
                pool: pool_idx,
                to: a_idx,
                in_token: pool.token_b,
            });
        }

        Self {
            tokens,
            pools,
            token_index,
            adj,
        }
    }

    pub fn num_tokens(&self) -> usize {
        self.tokens.len()
    }

    pub fn num_pools(&self) -> usize {
        self.pools.len()
    }

    pub fn index_of(&self, address: Address) -> Option<usize> {
        self.token_index.get(&address).copied()
    }

    pub fn address_of(&self, idx: usize) -> Address {
        self.tokens[idx].address
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::token::TokenKind;
    use alloy_primitives::U256;

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

    fn pool(pool_byte: u8, a: Address, b: Address) -> Pool {
        Pool {
            address: addr(pool_byte),
            token_a: a,
            token_b: b,
            reserve_a: U256::from(1_000u64),
            reserve_b: U256::from(1_000u64),
            fee_bps: 30,
            venue: "Test".to_string(),
        }
    }

    #[test]
    fn each_pool_contributes_exactly_two_directed_edges() {
        let tokens = vec![tok(1, "A"), tok(2, "B"), tok(3, "C")];
        let pools = vec![pool(0xA0, addr(1), addr(2)), pool(0xA1, addr(2), addr(3))];
        let g = Graph::new(tokens, pools);
        let total_edges: usize = g.adj.iter().map(|v| v.len()).sum();
        assert_eq!(total_edges, g.num_pools() * 2);
    }

    #[test]
    fn adj_lookup_returns_outgoing_edges_for_source() {
        let tokens = vec![tok(1, "A"), tok(2, "B"), tok(3, "C")];
        let pools = vec![pool(0xA0, addr(1), addr(2)), pool(0xA1, addr(1), addr(3))];
        let g = Graph::new(tokens, pools);
        let a = g.index_of(addr(1)).unwrap();
        let out_from_a = &g.adj[a];
        assert_eq!(out_from_a.len(), 2);
        let destinations: Vec<usize> = out_from_a.iter().map(|e| e.to).collect();
        assert!(destinations.contains(&g.index_of(addr(2)).unwrap()));
        assert!(destinations.contains(&g.index_of(addr(3)).unwrap()));
    }

    #[test]
    fn edges_are_directed_with_correct_in_token() {
        let tokens = vec![tok(1, "A"), tok(2, "B")];
        let pools = vec![pool(0xA0, addr(1), addr(2))];
        let g = Graph::new(tokens, pools);
        let a = g.index_of(addr(1)).unwrap();
        let b = g.index_of(addr(2)).unwrap();

        let edge_ab = g.adj[a].iter().find(|e| e.to == b).unwrap();
        assert_eq!(edge_ab.in_token, addr(1));

        let edge_ba = g.adj[b].iter().find(|e| e.to == a).unwrap();
        assert_eq!(edge_ba.in_token, addr(2));
    }

    #[test]
    fn disconnected_token_has_no_outgoing_edges() {
        let tokens = vec![tok(1, "A"), tok(2, "B"), tok(3, "C")];
        let pools = vec![pool(0xA0, addr(1), addr(2))]; // C is isolated
        let g = Graph::new(tokens, pools);
        let c = g.index_of(addr(3)).unwrap();
        assert!(g.adj[c].is_empty());
    }

    #[test]
    fn token_index_round_trips() {
        let tokens = vec![tok(1, "A"), tok(5, "E"), tok(9, "I")];
        let pools = vec![];
        let g = Graph::new(tokens.clone(), pools);
        for (i, t) in tokens.iter().enumerate() {
            assert_eq!(g.index_of(t.address), Some(i));
            assert_eq!(g.address_of(i), t.address);
        }
    }

    #[test]
    fn parallel_pools_between_same_pair_produce_four_edges() {
        // Two different pools (different fee tiers) on the same A/B pair
        // should show up as two parallel edges in each direction.
        let tokens = vec![tok(1, "A"), tok(2, "B")];
        let pools = vec![pool(0xA0, addr(1), addr(2)), pool(0xA1, addr(1), addr(2))];
        let g = Graph::new(tokens, pools);
        let a = g.index_of(addr(1)).unwrap();
        let b = g.index_of(addr(2)).unwrap();
        assert_eq!(g.adj[a].len(), 2);
        assert_eq!(g.adj[b].len(), 2);
        let distinct_pools: Vec<usize> = g.adj[a].iter().map(|e| e.pool).collect();
        assert_ne!(distinct_pools[0], distinct_pools[1]);
    }

    #[test]
    #[should_panic(expected = "identical tokens")]
    fn self_loop_pool_is_rejected() {
        let tokens = vec![tok(1, "A")];
        let pools = vec![pool(0xA0, addr(1), addr(1))];
        Graph::new(tokens, pools);
    }

    #[test]
    #[should_panic(expected = "unknown token")]
    fn pool_referencing_unknown_token_is_rejected() {
        let tokens = vec![tok(1, "A")];
        let pools = vec![pool(0xA0, addr(1), addr(99))]; // addr(99) not in tokens
        Graph::new(tokens, pools);
    }

    #[test]
    #[should_panic(expected = "duplicate token address")]
    fn duplicate_token_address_is_rejected() {
        let tokens = vec![tok(1, "A"), tok(1, "A_dup")];
        let pools = vec![];
        Graph::new(tokens, pools);
    }

    #[test]
    fn empty_graph_is_valid() {
        let g = Graph::new(vec![], vec![]);
        assert_eq!(g.num_tokens(), 0);
        assert_eq!(g.num_pools(), 0);
        assert!(g.adj.is_empty());
    }

    #[test]
    fn step_serde_round_trips() {
        use crate::trace::Step;
        let steps = vec![
            Step::Visit(3),
            Step::Relax {
                from: 1,
                to: 2,
                new_distance: -0.5,
            },
            Step::Pass(0),
        ];
        let json = serde_json::to_string(&steps).unwrap();
        let back: Vec<Step> = serde_json::from_str(&json).unwrap();
        assert_eq!(steps, back);
    }
}
