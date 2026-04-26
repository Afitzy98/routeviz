use serde::{Deserialize, Serialize};

// Step vocabulary for algorithm traces. Nodes are referred to by their dense
// `usize` index into `Graph.tokens`; the WASM boundary translates these back
// to `Address` when serialising for the UI. Keeping the trace in usize form
// internally makes algorithm inner loops cheap.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Step {
    Visit(usize),
    Relax {
        from: usize,
        to: usize,
        new_distance: f64,
    },
    Pass(usize),
}
