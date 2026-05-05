use std::collections::{HashMap, HashSet, VecDeque};

use alloy_primitives::U256;

use crate::graph::Graph;

use super::hub::{CoreGraphView, RealEdge};
use super::path::DiscoveredPath;
use super::shortcut::{Shortcut, ShortcutIndex};
use super::util::u256_to_f64;

// Cap on total queue pops to bound worst-case work even when dominance
// pruning is weak (uniform graphs etc).
const SPFA_POP_LIMIT: usize = 50_000;

#[derive(Debug, Clone)]
enum EdgeStep {
    Real(RealEdge),
    Shortcut { idx: usize },
}

// SPFA over the core graph + shortcut virtual edges, with dominance
// pruning on per-token best amount-out. Returns the disjoint path whose
// realised flow ratio is the largest *and* strictly exceeds τ.
#[allow(clippy::too_many_arguments)]
pub fn find_path(
    graph: &Graph,
    core: &CoreGraphView<'_>,
    shortcuts: &ShortcutIndex,
    s: usize,
    t: usize,
    amount_in: U256,
    tau: f64,
    used_pools: &HashSet<usize>,
    max_hops: usize,
) -> Option<DiscoveredPath> {
    if amount_in.is_zero() || s == t {
        return None;
    }

    // Stable index per shortcut so EdgeStep::Shortcut can refer to it.
    let all_shortcuts: Vec<&Shortcut> = shortcuts.by_pair.values().flat_map(|v| v.iter()).collect();
    let shortcuts_by_from: HashMap<usize, Vec<usize>> = {
        let mut m: HashMap<usize, Vec<usize>> = HashMap::new();
        for (i, sc) in all_shortcuts.iter().enumerate() {
            m.entry(sc.from_hub).or_default().push(i);
        }
        m
    };

    // best[v] = max amount-out reached at v so far. Anything ≤ best[v] is
    // dominance-pruned.
    let mut best: HashMap<usize, U256> = HashMap::new();
    best.insert(s, amount_in);

    // Queue: (current_token, amount_at_current, edge_steps_so_far, hop_count).
    let mut queue: VecDeque<(usize, U256, Vec<EdgeStep>, usize)> = VecDeque::new();
    queue.push_back((s, amount_in, Vec::new(), 0));

    let mut best_completed: Option<DiscoveredPath> = None;
    let mut best_completed_amount = U256::ZERO;
    // Admission threshold on actual flow ratio (amount_at_t / amount_in).
    let mut tau_floor = tau;

    let amount_in_f = u256_to_f64(amount_in);
    let mut pops = 0usize;

    while let Some((u, amount_at_u, steps, hops)) = queue.pop_front() {
        pops += 1;
        if pops > SPFA_POP_LIMIT {
            break;
        }

        if u == t {
            let amount_f = u256_to_f64(amount_at_u);
            let mp = if amount_in_f > 0.0 {
                amount_f / amount_in_f
            } else {
                0.0
            };
            if mp > tau_floor && amount_at_u > best_completed_amount {
                best_completed_amount = amount_at_u;
                tau_floor = mp;
                best_completed = Some(materialize(&steps, s, &all_shortcuts));
            }
            continue;
        }

        if hops >= max_hops {
            continue;
        }

        // Visited-tokens set for simple-path constraint.
        let visited_tokens: HashSet<usize> = {
            let mut v = HashSet::new();
            v.insert(s);
            for step in &steps {
                match step {
                    EdgeStep::Real(re) => {
                        v.insert(re.to);
                    }
                    EdgeStep::Shortcut { idx } => {
                        let sc = &all_shortcuts[*idx];
                        for &it in &sc.intermediate_tokens {
                            v.insert(it);
                        }
                        v.insert(sc.to_hub);
                    }
                }
            }
            v
        };

        // Real edges out of u in the core graph.
        for re in core.edges_out(u) {
            if used_pools.contains(&re.pool) || visited_tokens.contains(&re.to) {
                continue;
            }
            let pool = &graph.pools[re.pool];
            let out = pool.output_amount(re.in_token, amount_at_u);
            if out.is_zero() {
                continue;
            }
            let prev = best.get(&re.to).copied().unwrap_or(U256::ZERO);
            if out > prev {
                best.insert(re.to, out);
                let mut new_steps = steps.clone();
                new_steps.push(EdgeStep::Real(re));
                queue.push_back((re.to, out, new_steps, hops + 1));
            }
        }

        // Shortcut virtual edges, only if u is a hub.
        if core.hubs.contains(u)
            && let Some(idxs) = shortcuts_by_from.get(&u)
        {
            for &sc_idx in idxs {
                let sc = &all_shortcuts[sc_idx];
                // Skip if any underlying pool is taken (cross-path) or
                // visited inside the path being built (cycle).
                if sc.pools.iter().any(|p| used_pools.contains(p)) {
                    continue;
                }
                if sc
                    .intermediate_tokens
                    .iter()
                    .any(|t| visited_tokens.contains(t))
                    || visited_tokens.contains(&sc.to_hub)
                {
                    continue;
                }
                let out = sc.simulate(graph, amount_at_u);
                if out.is_zero() {
                    continue;
                }
                let new_hops = hops + sc.pools.len();
                if new_hops > max_hops {
                    continue;
                }
                let target = sc.to_hub;
                let prev = best.get(&target).copied().unwrap_or(U256::ZERO);
                if out > prev {
                    best.insert(target, out);
                    let mut new_steps = steps.clone();
                    new_steps.push(EdgeStep::Shortcut { idx: sc_idx });
                    queue.push_back((target, out, new_steps, new_hops));
                }
            }
        }
    }

    best_completed
}

// Expand recorded EdgeSteps into a flat (tokens, pools) DiscoveredPath.
// Shortcut steps unfold their underlying intermediates and pool sequence.
fn materialize(steps: &[EdgeStep], s: usize, all_shortcuts: &[&Shortcut]) -> DiscoveredPath {
    let mut tokens = vec![s];
    let mut pools = Vec::new();
    for step in steps {
        match step {
            EdgeStep::Real(re) => {
                tokens.push(re.to);
                pools.push(re.pool);
            }
            EdgeStep::Shortcut { idx } => {
                let sc = all_shortcuts[*idx];
                for &it in &sc.intermediate_tokens {
                    tokens.push(it);
                }
                tokens.push(sc.to_hub);
                pools.extend_from_slice(&sc.pools);
            }
        }
    }
    DiscoveredPath { tokens, pools }
}
