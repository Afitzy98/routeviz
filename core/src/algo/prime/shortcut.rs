use std::collections::{HashMap, VecDeque};

use alloy_primitives::U256;

use crate::algo::path::walk_pool_path;
use crate::graph::Graph;

use super::config::PrimeConfig;
use super::hub::HubSet;
use super::sim::marginal_price_at_zero;

#[derive(Debug, Clone)]
pub struct Shortcut {
    pub from_hub: usize,
    pub to_hub: usize,
    /// Non-hub tokens visited (ordered).
    pub intermediate_tokens: Vec<usize>,
    /// Pool indices, one per hop (length = intermediate_tokens.len() + 1).
    pub pools: Vec<usize>,
    /// Cached zero-flow marginal price for ranking.
    pub mp_at_zero: f64,
}

impl Shortcut {
    pub fn full_token_path(&self) -> Vec<usize> {
        let mut v = Vec::with_capacity(self.intermediate_tokens.len() + 2);
        v.push(self.from_hub);
        v.extend(self.intermediate_tokens.iter().copied());
        v.push(self.to_hub);
        v
    }

    pub fn simulate(&self, graph: &Graph, amount_in: U256) -> U256 {
        let tokens = self.full_token_path();
        match walk_pool_path(graph, &tokens, &self.pools, amount_in) {
            Some((out, _)) => out,
            None => U256::ZERO,
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct ShortcutIndex {
    pub by_pair: HashMap<(usize, usize), Vec<Shortcut>>,
}

impl ShortcutIndex {
    pub fn shortcuts_from(&self, hub: usize) -> impl Iterator<Item = &Shortcut> {
        self.by_pair
            .iter()
            .filter_map(move |((from, _to), v)| if *from == hub { Some(v.iter()) } else { None })
            .flatten()
    }
}

// For each ordered hub pair, BFS through non-hub intermediates only and
// keep the top-N shortcuts by zero-flow marginal price.
pub fn build_shortcut_index(graph: &Graph, hubs: &HubSet, config: &PrimeConfig) -> ShortcutIndex {
    let mut by_pair: HashMap<(usize, usize), Vec<Shortcut>> = HashMap::new();
    let max_inter = config.shortcut_max_intermediates;
    let top_n = config.shortcuts_per_hub_pair;

    for &start in &hubs.hubs {
        // BFS state: (current_token, intermediates_so_far, pools_so_far).
        let mut queue: VecDeque<(usize, Vec<usize>, Vec<usize>)> = VecDeque::new();
        queue.push_back((start, Vec::new(), Vec::new()));

        while let Some((u, inters, pools)) = queue.pop_front() {
            for edge in &graph.adj[u] {
                let v = edge.to;
                if v == start || inters.contains(&v) || pools.contains(&edge.pool) {
                    continue;
                }

                if hubs.contains(v) {
                    // Reached another hub — record shortcut (length ≥ 2).
                    if !inters.is_empty() {
                        let mut new_pools = pools.clone();
                        new_pools.push(edge.pool);
                        let tokens: Vec<usize> = std::iter::once(start)
                            .chain(inters.iter().copied())
                            .chain(std::iter::once(v))
                            .collect();
                        let mp = marginal_price_at_zero(graph, &tokens, &new_pools);
                        if mp.is_finite() && mp > 0.0 {
                            by_pair.entry((start, v)).or_default().push(Shortcut {
                                from_hub: start,
                                to_hub: v,
                                intermediate_tokens: inters.clone(),
                                pools: new_pools,
                                mp_at_zero: mp,
                            });
                        }
                    }
                    // Don't continue BFS through hubs — shortcuts only have
                    // non-hub intermediates.
                } else if inters.len() < max_inter {
                    let mut new_inters = inters.clone();
                    new_inters.push(v);
                    let mut new_pools = pools.clone();
                    new_pools.push(edge.pool);
                    queue.push_back((v, new_inters, new_pools));
                }
            }
        }
    }

    for shortcuts in by_pair.values_mut() {
        shortcuts.sort_by(|a, b| {
            b.mp_at_zero
                .partial_cmp(&a.mp_at_zero)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        shortcuts.truncate(top_n);
    }

    ShortcutIndex { by_pair }
}
