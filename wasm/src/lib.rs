use alloy_primitives::{Address, U256};
use routeviz_core::algo::{self, Algorithm, GasModel, SolveOptions, arb_scan};
use routeviz_core::generator::{GenConfig, PoolGenerator};
use routeviz_core::graph::Graph;
use routeviz_core::layout::{Point, fruchterman_reingold_layout, hub_spoke_layout};
use routeviz_core::pool::Pool;
use routeviz_core::token::{Token, TokenKind};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use wasm_bindgen::prelude::*;

// Generator output + canvas positions. U256 serialises as 0x-string
// (JS decodes via BigInt).
#[derive(Serialize)]
struct GeneratedGraph {
    tokens: Vec<Token>,
    pools: Vec<Pool>,
    positions: Vec<Point>,
    config: GenConfig,
}

const HUB_RING_RADIUS: f64 = 140.0;
const SPOKE_RING_RADIUS: f64 = 420.0;

#[wasm_bindgen]
pub fn generate_graph(config_js: JsValue) -> Result<JsValue, JsValue> {
    let config: GenConfig = serde_wasm_bindgen::from_value(config_js)
        .map_err(|e| JsValue::from_str(&format!("invalid config: {e}")))?;
    let mut generator = PoolGenerator::new(config.clone());
    let (tokens, pools) = generator.generate();
    let is_hub: Vec<bool> = tokens
        .iter()
        .map(|t| matches!(t.kind, TokenKind::Hub))
        .collect();
    let positions = hub_spoke_layout(&is_hub, HUB_RING_RADIUS, SPOKE_RING_RADIUS);
    let response = GeneratedGraph {
        tokens,
        pools,
        positions,
        config,
    };
    serde_wasm_bindgen::to_value(&response)
        .map_err(|e| JsValue::from_str(&format!("serialisation failed: {e}")))
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum LayoutMode {
    HubSpoke,
    ForceDirected,
}

#[derive(Deserialize)]
struct RelayoutRequest {
    tokens: Vec<Token>,
    pools: Vec<Pool>,
    layout: LayoutMode,
    seed: u64,
}

#[wasm_bindgen]
pub fn relayout(request_js: JsValue) -> Result<JsValue, JsValue> {
    let req: RelayoutRequest = serde_wasm_bindgen::from_value(request_js)
        .map_err(|e| JsValue::from_str(&format!("invalid relayout request: {e}")))?;

    let positions = match req.layout {
        LayoutMode::HubSpoke => {
            let is_hub: Vec<bool> = req
                .tokens
                .iter()
                .map(|t| matches!(t.kind, TokenKind::Hub))
                .collect();
            hub_spoke_layout(&is_hub, HUB_RING_RADIUS, SPOKE_RING_RADIUS)
        }
        LayoutMode::ForceDirected => {
            // Build undirected adjacency by token index from the pool set.
            let token_idx: HashMap<Address, usize> = req
                .tokens
                .iter()
                .enumerate()
                .map(|(i, t)| (t.address, i))
                .collect();
            let mut adjacency: Vec<Vec<usize>> = vec![Vec::new(); req.tokens.len()];
            for pool in &req.pools {
                let (Some(&a), Some(&b)) =
                    (token_idx.get(&pool.token_a), token_idx.get(&pool.token_b))
                else {
                    continue;
                };
                if !adjacency[a].contains(&b) {
                    adjacency[a].push(b);
                }
                if !adjacency[b].contains(&a) {
                    adjacency[b].push(a);
                }
            }
            // Bounding box ~matches the hub-spoke layout's diameter so
            // the canvas scale stays roughly comparable across modes.
            fruchterman_reingold_layout(
                req.tokens.len(),
                &adjacency,
                SPOKE_RING_RADIUS * 2.0,
                80,
                req.seed,
            )
        }
    };

    serde_wasm_bindgen::to_value(&positions)
        .map_err(|e| JsValue::from_str(&format!("serialisation failed: {e}")))
}

#[wasm_bindgen]
pub fn default_config() -> Result<JsValue, JsValue> {
    serde_wasm_bindgen::to_value(&GenConfig::default())
        .map_err(|e| JsValue::from_str(&e.to_string()))
}

// Frontend round-trips tokens/pools instead of keeping WASM state.
#[derive(Deserialize)]
struct SolveRequest {
    algo: Algorithm,
    tokens: Vec<Token>,
    pools: Vec<Pool>,
    src: Address,
    dst: Address,
    amount_in: U256,
    #[serde(default = "default_true")]
    with_trace: bool,
    #[serde(default)]
    gas_price_gwei: f64,
}

fn default_true() -> bool {
    true
}

#[wasm_bindgen]
pub fn solve(request_js: JsValue) -> Result<JsValue, JsValue> {
    let req: SolveRequest = serde_wasm_bindgen::from_value(request_js)
        .map_err(|e| JsValue::from_str(&format!("invalid solve request: {e}")))?;
    let graph = Graph::new(req.tokens, req.pools);
    let opts = SolveOptions {
        with_trace: req.with_trace,
        gas: GasModel::at_gwei(req.gas_price_gwei),
    };
    let result = algo::solve_with_opts(req.algo, &graph, req.src, req.dst, req.amount_in, opts);
    serde_wasm_bindgen::to_value(&result)
        .map_err(|e| JsValue::from_str(&format!("serialisation failed: {e}")))
}

#[derive(Deserialize)]
struct ScanArbRequest {
    tokens: Vec<Token>,
    pools: Vec<Pool>,
    entry: Address,
    #[serde(default)]
    gas_price_gwei: f64,
}

#[wasm_bindgen]
pub fn scan_arb(request_js: JsValue) -> Result<JsValue, JsValue> {
    let req: ScanArbRequest = serde_wasm_bindgen::from_value(request_js)
        .map_err(|e| JsValue::from_str(&format!("invalid scan_arb request: {e}")))?;
    let graph = Graph::new(req.tokens, req.pools);
    let gas = GasModel::at_gwei(req.gas_price_gwei);
    let result = arb_scan::scan_from(&graph, req.entry, &gas);
    serde_wasm_bindgen::to_value(&result)
        .map_err(|e| JsValue::from_str(&format!("serialisation failed: {e}")))
}

#[derive(Deserialize)]
struct InjectArbRequest {
    pools: Vec<Pool>,
    magnitude: f64,
    seed: u64,
}

// Mutate one random pool to create a detectable arb. Deterministic.
#[wasm_bindgen]
pub fn inject_arb(request_js: JsValue) -> Result<JsValue, JsValue> {
    let req: InjectArbRequest = serde_wasm_bindgen::from_value(request_js)
        .map_err(|e| JsValue::from_str(&format!("invalid inject_arb request: {e}")))?;
    if req.pools.is_empty() {
        return Err(JsValue::from_str(
            "cannot inject arb into an empty pool set",
        ));
    }
    let mut pools = req.pools;
    let mut pg = PoolGenerator::new(GenConfig {
        seed: req.seed,
        ..GenConfig::default()
    });
    pg.inject_arb(&mut pools, req.magnitude);
    serde_wasm_bindgen::to_value(&pools)
        .map_err(|e| JsValue::from_str(&format!("serialisation failed: {e}")))
}
