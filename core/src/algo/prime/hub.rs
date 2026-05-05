use std::collections::{HashMap, HashSet};

use alloy_primitives::Address;

use crate::graph::Graph;

use super::util::u256_to_f64;

#[derive(Debug, Clone)]
pub struct HubSet {
    pub hubs: HashSet<usize>,
    /// Hub → rank by liquidity (0 = highest). Useful for tie-breaking.
    pub rank: HashMap<usize, u32>,
}

impl HubSet {
    pub fn contains(&self, idx: usize) -> bool {
        self.hubs.contains(&idx)
    }
}

// Score = Σ_pools (reserve_in_token / 10^decimals) × token.true_price_usd
fn token_liquidity_score(graph: &Graph, token_idx: usize) -> f64 {
    let token = &graph.tokens[token_idx];
    let scale = 10f64.powi(token.decimals as i32);
    let mut score = 0.0;
    for pool in &graph.pools {
        let reserve = if pool.token_a == token.address {
            pool.reserve_a
        } else if pool.token_b == token.address {
            pool.reserve_b
        } else {
            continue;
        };
        score += u256_to_f64(reserve) / scale * token.true_price_usd;
    }
    score
}

// Top-K tokens by aggregate liquidity. Deterministic tie-break by token index.
pub fn build_hub_set(graph: &Graph, k: usize) -> HubSet {
    let n = graph.num_tokens();
    let mut scored: Vec<(usize, f64)> = (0..n)
        .map(|i| (i, token_liquidity_score(graph, i)))
        .collect();
    scored.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.0.cmp(&b.0))
    });
    let take = k.min(n);
    let mut hubs = HashSet::with_capacity(take);
    let mut rank = HashMap::with_capacity(take);
    for (r, (idx, _)) in scored.into_iter().take(take).enumerate() {
        hubs.insert(idx);
        rank.insert(idx, r as u32);
    }
    HubSet { hubs, rank }
}

// View over the original graph that exposes only edges where both
// endpoints lie in (hubs ∪ {s, t}). Filtering at iteration time — no clone.
pub struct CoreGraphView<'g> {
    pub graph: &'g Graph,
    pub hubs: &'g HubSet,
    pub s: usize,
    pub t: usize,
}

impl<'g> CoreGraphView<'g> {
    fn allowed(&self, idx: usize) -> bool {
        idx == self.s || idx == self.t || self.hubs.contains(idx)
    }

    pub fn edges_out(&self, u: usize) -> impl Iterator<Item = RealEdge> + '_ {
        self.graph.adj[u].iter().filter_map(move |edge| {
            if !self.allowed(edge.to) {
                return None;
            }
            Some(RealEdge {
                pool: edge.pool,
                from: u,
                to: edge.to,
                in_token: edge.in_token,
            })
        })
    }
}

#[derive(Debug, Clone, Copy)]
pub struct RealEdge {
    pub pool: usize,
    pub from: usize,
    pub to: usize,
    pub in_token: Address,
}
