use std::collections::HashMap;

use crate::graph::Graph;

use super::path::MultiEdgePath;

// Group input paths by token sequence; for each group, union the per-hop
// pools and expand with all other pools between the same token pair at
// each hop. Initial edge weights are uniform within each hop.
//
// On pool-disjointness: the paper's disjoint constraint applies *between*
// distinct paths in P, not *within* an expanded multi-edge path. Once
// paths merge by token sequence, parallel pools at a hop are alternatives
// for the same merged path's flow and can coexist.
pub fn merge_and_expand(
    single_paths: Vec<(MultiEdgePath, f64)>,
    graph: &Graph,
) -> Vec<MultiEdgePath> {
    type Group = (Vec<usize>, Vec<(MultiEdgePath, f64)>);
    let mut groups: Vec<Group> = Vec::new();
    'outer: for (path, alloc) in single_paths {
        for (key, group) in groups.iter_mut() {
            if *key == path.tokens {
                group.push((path, alloc));
                continue 'outer;
            }
        }
        groups.push((path.tokens.clone(), vec![(path, alloc)]));
    }

    // Pre-index pools by ordered token pair for O(1) "pools between" lookup.
    let mut pools_between: HashMap<(usize, usize), Vec<usize>> = HashMap::new();
    for (i, pool) in graph.pools.iter().enumerate() {
        let Some(a) = graph.index_of(pool.token_a) else {
            continue;
        };
        let Some(b) = graph.index_of(pool.token_b) else {
            continue;
        };
        pools_between.entry((a, b)).or_default().push(i);
        pools_between.entry((b, a)).or_default().push(i);
    }

    let mut merged: Vec<MultiEdgePath> = Vec::with_capacity(groups.len());
    for (tokens, group) in groups {
        let n_hops = tokens.len() - 1;
        let mut pools_at_hop: Vec<Vec<usize>> = (0..n_hops).map(|_| Vec::new()).collect();
        let mut alloc_total = 0.0f64;

        // Union of pools per hop across the group's paths.
        for (path, alloc) in &group {
            alloc_total += alloc;
            for (h, pools) in path.pools_at_hop.iter().enumerate() {
                for &p in pools {
                    if !pools_at_hop[h].contains(&p) {
                        pools_at_hop[h].push(p);
                    }
                }
            }
        }

        // Expand: add any other pools between the same token pair.
        for h in 0..n_hops {
            let key = (tokens[h], tokens[h + 1]);
            if let Some(extras) = pools_between.get(&key) {
                for &p in extras {
                    if !pools_at_hop[h].contains(&p) {
                        pools_at_hop[h].push(p);
                    }
                }
            }
        }

        // Initial edge weights: uniform within each hop.
        let edge_weights: Vec<Vec<f64>> = pools_at_hop
            .iter()
            .map(|pools| {
                let n = pools.len().max(1);
                vec![1.0 / n as f64; n]
            })
            .collect();

        merged.push(MultiEdgePath {
            tokens,
            pools_at_hop,
            edge_weights,
            alloc: alloc_total,
        });
    }

    // Renormalise allocations against float drift from the source group.
    let alloc_sum: f64 = merged.iter().map(|p| p.alloc).sum();
    if alloc_sum > 1e-12 {
        for m in merged.iter_mut() {
            m.alloc /= alloc_sum;
        }
    }

    merged
}
