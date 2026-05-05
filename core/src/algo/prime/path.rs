// Path types used across the PRIME pipeline.
//
// `DiscoveredPath` is the output of FindPath — a single concrete pool per
// hop. `MultiEdgePath` is the working representation: pre-MergeAndExpand
// each `pools_at_hop[h]` is a single-element vec; post-merge it can carry
// the parallel pools at that hop with `edge_weights[h]` as their split.

#[derive(Debug, Clone)]
pub struct DiscoveredPath {
    pub tokens: Vec<usize>,
    pub pools: Vec<usize>,
}

#[derive(Debug, Clone)]
pub struct MultiEdgePath {
    pub tokens: Vec<usize>,
    /// pools_at_hop[h] = pool indices usable at hop h. Length 1 before
    /// MergeAndExpand; may be > 1 after.
    pub pools_at_hop: Vec<Vec<usize>>,
    /// edge_weights[h][i] = fraction of hop-h input flowing through
    /// pools_at_hop[h][i]. Σ_i edge_weights[h][i] = 1.0.
    pub edge_weights: Vec<Vec<f64>>,
    /// Allocation share of total input on this path. Σ over paths = 1.
    pub alloc: f64,
}

impl MultiEdgePath {
    pub fn from_single(tokens: Vec<usize>, pools: Vec<usize>, alloc: f64) -> Self {
        let pools_at_hop: Vec<Vec<usize>> = pools.iter().map(|&p| vec![p]).collect();
        let edge_weights: Vec<Vec<f64>> = pools_at_hop.iter().map(|_| vec![1.0]).collect();
        Self {
            tokens,
            pools_at_hop,
            edge_weights,
            alloc,
        }
    }

    pub fn collect_pools(&self) -> Vec<usize> {
        let mut out = Vec::new();
        for hop in &self.pools_at_hop {
            out.extend_from_slice(hop);
        }
        out
    }
}
