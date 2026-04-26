use alloy_primitives::{Address, U256};
use serde::{Deserialize, Serialize};

use crate::graph::Graph;
use crate::trace::Step;

pub mod amount_aware;
pub mod arb_scan;
pub mod bellman_ford;
pub mod bounded_bf;
pub mod dijkstra;
pub mod gas;
pub mod path;
pub mod split_common;
pub mod split_dp;
pub mod split_fw;

pub use gas::GasModel;

#[cfg(test)]
pub mod testkit;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Algorithm {
    Dijkstra,
    BellmanFord,
    AmountAware,
    SplitDp,
    SplitFw,
}

// One leg of a split-routing result. amount_in across legs sums to the
// user's total; amount_out across legs sums to the realised output.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Leg {
    pub path: Vec<Address>,
    pub pools_used: Vec<Address>,
    pub amount_in: U256,
    pub amount_out: U256,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Outcome {
    Found {
        path: Vec<Address>,
        pools_used: Vec<Address>,
        total_log_weight: f64,
        product_of_rates: f64,
        amount_in: U256,
        /// Gross output before gas. Net = amount_out - gas_cost.
        amount_out: U256,
        /// Cost in dst-token base units; zero when gas is disabled.
        #[serde(default)]
        gas_cost: U256,
    },
    FoundSplit {
        legs: Vec<Leg>,
        amount_in: U256,
        amount_out: U256,
        #[serde(default)]
        gas_cost: U256,
    },
    NegativeCycle {
        cycle: Vec<Address>,
        pools_used: Vec<Address>,
        product_of_rates: f64,
        amount_in: U256,
        cycle_output: U256,
        #[serde(default)]
        gas_cost: U256,
    },
    NoPath,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SolveResult {
    pub outcome: Outcome,
    pub trace: Vec<Step>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SolveOptions {
    /// Disables trace collection. Cheap on big graphs.
    pub with_trace: bool,
    /// When enabled, algorithms optimise net of gas instead of gross.
    pub gas: GasModel,
}

impl Default for SolveOptions {
    fn default() -> Self {
        Self {
            with_trace: true,
            gas: GasModel::off(),
        }
    }
}

// `push` is a no-op when disabled — saves big on Dijkstra/BF at large V.
#[derive(Debug)]
pub struct Tracer(Option<Vec<Step>>);

impl Tracer {
    pub fn new(enabled: bool) -> Self {
        Self(if enabled { Some(Vec::new()) } else { None })
    }

    pub fn with_capacity(enabled: bool, cap: usize) -> Self {
        Self(if enabled {
            Some(Vec::with_capacity(cap))
        } else {
            None
        })
    }

    pub fn push(&mut self, step: Step) {
        if let Some(v) = &mut self.0 {
            v.push(step);
        }
    }

    pub fn into_vec(self) -> Vec<Step> {
        self.0.unwrap_or_default()
    }
}

pub fn solve(
    algo: Algorithm,
    graph: &Graph,
    src: Address,
    dst: Address,
    amount_in: U256,
) -> SolveResult {
    solve_with_opts(algo, graph, src, dst, amount_in, SolveOptions::default())
}

pub fn solve_with_opts(
    algo: Algorithm,
    graph: &Graph,
    src: Address,
    dst: Address,
    amount_in: U256,
    opts: SolveOptions,
) -> SolveResult {
    match algo {
        Algorithm::Dijkstra => {
            dijkstra::solve(graph, src, dst, amount_in, opts.with_trace, &opts.gas)
        }
        Algorithm::BellmanFord => {
            bellman_ford::solve(graph, src, dst, amount_in, opts.with_trace, &opts.gas)
        }
        Algorithm::AmountAware => {
            amount_aware::solve(graph, src, dst, amount_in, opts.with_trace, &opts.gas)
        }
        Algorithm::SplitDp => {
            split_dp::solve(graph, src, dst, amount_in, opts.with_trace, &opts.gas)
        }
        Algorithm::SplitFw => {
            split_fw::solve(graph, src, dst, amount_in, opts.with_trace, &opts.gas)
        }
    }
}
