use alloy_primitives::U256;

use crate::algo::split_common::fractional_mul;
use crate::graph::Graph;

use super::config::AsgmConfig;
use super::path::MultiEdgePath;
use super::sim::{
    amount_entering_hop, evaluate_gross, marginal_price_path, simulate_path_isolated,
};
use super::util::u256_to_f64;

// ASGM: Away-Step Gradient Method. Updates path allocations and per-hop
// edge weights in place. Returns τ — the equilibrium marginal price after
// convergence — which Stage 1 uses as the next admission threshold.
//
// Armijo sufficient-decrease (paper §4.2):
//   J(W + Δ·SIGN) ≥ J(W) + α·Δ·x·(g_{p+} − g_{p−})
// The `x` factor (x = amount_in in base units) is essential: without it
// the threshold is near-zero for any token amount, so Armijo accepts
// non-improving steps and the line search degenerates.
//
// Marginal rates use BASE pool reserves (paper-faithful), not post-flow
// reserves. The path's f' at the current operating point is derivative
// of the original swap function evaluated at the path's current input.
pub fn run_asgm(
    paths: &mut [MultiEdgePath],
    graph: &Graph,
    amount_in: U256,
    cfg: &AsgmConfig,
) -> f64 {
    if paths.is_empty() {
        return 0.0;
    }
    let x_f = u256_to_f64(amount_in);

    if paths.len() == 1 && paths[0].pools_at_hop.iter().all(|h| h.len() <= 1) {
        // Nothing to optimise — single path, single pool per hop.
        let flow = fractional_mul(amount_in, paths[0].alloc);
        let mp = marginal_price_path(graph, &paths[0], flow);
        return if mp.is_finite() { mp } else { 0.0 };
    }

    for _ in 0..cfg.max_iter {
        // Inner pass: optimise edge weights within each merged path.
        let mut inner_progress = false;
        #[allow(clippy::needless_range_loop)]
        for path_idx in 0..paths.len() {
            if paths[path_idx].alloc <= 0.0 {
                continue;
            }
            let path_amount_in = fractional_mul(amount_in, paths[path_idx].alloc);
            for hop in 0..paths[path_idx].pools_at_hop.len() {
                if paths[path_idx].pools_at_hop[hop].len() < 2 {
                    continue;
                }
                // Recompute f0 per hop — earlier hops in this pass may have
                // already shifted weights, so the path's output drifted.
                let f0_path_f = u256_to_f64(simulate_path_isolated(
                    graph,
                    &paths[path_idx],
                    path_amount_in,
                ));
                let amount_at_hop =
                    amount_entering_hop(graph, &paths[path_idx], hop, path_amount_in);
                if asgm_inner_step(
                    &mut paths[path_idx],
                    hop,
                    graph,
                    amount_at_hop,
                    path_amount_in,
                    cfg,
                    f0_path_f,
                    x_f,
                ) {
                    inner_progress = true;
                }
            }
        }

        // Outer pass: shift allocation across paths.
        let outer_progress = asgm_outer_step(paths, graph, amount_in, cfg, x_f);

        if !inner_progress && !outer_progress {
            break;
        }
    }

    // τ = max marginal price across active paths at the converged allocation.
    paths
        .iter()
        .filter(|p| p.alloc > 0.0)
        .map(|p| {
            let flow = fractional_mul(amount_in, p.alloc);
            let mp = marginal_price_path(graph, p, flow);
            if mp.is_finite() { mp } else { 0.0 }
        })
        .fold(0.0_f64, f64::max)
}

// Outer step: shift allocation from path with min marginal price to path
// with max. Returns true if a non-trivial step was accepted.
fn asgm_outer_step(
    paths: &mut [MultiEdgePath],
    graph: &Graph,
    amount_in: U256,
    cfg: &AsgmConfig,
    x_f: f64,
) -> bool {
    if paths.len() < 2 {
        return false;
    }
    let f0_f = u256_to_f64(evaluate_gross(graph, paths, amount_in));

    let g: Vec<f64> = paths
        .iter()
        .map(|p| {
            let flow = fractional_mul(amount_in, p.alloc);
            let mp = marginal_price_path(graph, p, flow);
            if mp.is_finite() { mp } else { 0.0 }
        })
        .collect();

    let p_plus = arg_max(&g);
    let p_minus = arg_min(&g);
    let slope = g[p_plus] - g[p_minus];
    if slope < cfg.epsilon {
        return false;
    }

    let delta_max = paths[p_minus].alloc;
    let mut delta = cfg.armijo_delta_0.min(delta_max);
    if delta < cfg.armijo_min_delta {
        return false;
    }

    let orig: Vec<f64> = paths.iter().map(|p| p.alloc).collect();
    let accepted = loop {
        if delta < cfg.armijo_min_delta {
            break 0.0;
        }
        let mut trial = orig.clone();
        trial[p_plus] += delta;
        trial[p_minus] = (trial[p_minus] - delta).max(0.0);
        let sum: f64 = trial.iter().sum();
        if sum > 1e-12 {
            for c in trial.iter_mut() {
                *c /= sum;
            }
        }
        for (p, &c) in paths.iter_mut().zip(trial.iter()) {
            p.alloc = c;
        }
        let fc_f = u256_to_f64(evaluate_gross(graph, paths, amount_in));
        if fc_f >= f0_f + cfg.armijo_c * delta * x_f * slope {
            break delta;
        }
        for (p, &o) in paths.iter_mut().zip(orig.iter()) {
            p.alloc = o;
        }
        delta *= cfg.armijo_beta;
    };

    if accepted < cfg.armijo_min_delta {
        for (p, &o) in paths.iter_mut().zip(orig.iter()) {
            p.alloc = o;
        }
        return false;
    }
    true
}

// Inner step: optimise edge weights of one hop within one path. Returns
// true if a non-trivial weight shift was accepted.
#[allow(clippy::too_many_arguments)]
fn asgm_inner_step(
    path: &mut MultiEdgePath,
    hop: usize,
    graph: &Graph,
    amount_at_hop: U256,
    path_amount_in: U256,
    cfg: &AsgmConfig,
    f0_path_f: f64,
    x_f: f64,
) -> bool {
    if path.pools_at_hop[hop].len() < 2 {
        return false;
    }
    let weights = path.edge_weights[hop].clone();
    let pools = path.pools_at_hop[hop].clone();
    let from_addr = graph.tokens[path.tokens[hop]].address;

    // Per-pool marginal rate at its current share — base reserves.
    let g: Vec<f64> = pools
        .iter()
        .enumerate()
        .map(|(i, &pool_idx)| {
            let flow = fractional_mul(amount_at_hop, weights[i]);
            graph.pools[pool_idx].marginal_rate_at_flow(from_addr, flow)
        })
        .collect();

    let p_plus = arg_max(&g);
    let p_minus = arg_min(&g);
    let slope = g[p_plus] - g[p_minus];
    if slope < cfg.epsilon {
        return false;
    }

    let delta_max = weights[p_minus];
    let mut delta = cfg.armijo_delta_0.min(delta_max);
    if delta < cfg.armijo_min_delta {
        return false;
    }

    let orig = weights.clone();
    let accepted = loop {
        if delta < cfg.armijo_min_delta {
            break 0.0;
        }
        let mut trial = orig.clone();
        trial[p_plus] += delta;
        trial[p_minus] = (trial[p_minus] - delta).max(0.0);
        let sum: f64 = trial.iter().sum();
        if sum > 1e-12 {
            for c in trial.iter_mut() {
                *c /= sum;
            }
        }
        path.edge_weights[hop] = trial;
        // Evaluate the path's full output from its start input — hop output
        // feeds downstream hops, so partial-hop comparison would mislead.
        let fc_path_f = u256_to_f64(simulate_path_isolated(graph, path, path_amount_in));
        if fc_path_f >= f0_path_f + cfg.armijo_c * delta * x_f * slope {
            break delta;
        }
        path.edge_weights[hop] = orig.clone();
        delta *= cfg.armijo_beta;
    };

    if accepted < cfg.armijo_min_delta {
        path.edge_weights[hop] = orig;
        return false;
    }
    true
}

fn arg_max(g: &[f64]) -> usize {
    g.iter()
        .enumerate()
        .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(i, _)| i)
        .unwrap_or(0)
}

fn arg_min(g: &[f64]) -> usize {
    g.iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(i, _)| i)
        .unwrap_or(0)
}
