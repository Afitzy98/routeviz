# RouteViz

Interactive visualiser for DEX (decentralised-exchange) routing and arbitrage. Rust core, WASM-powered browser UI, side-by-side benchmarks across algorithms.

> **Live demo:** https://afitzy98.github.io/routeviz/

## What it shows

A synthetic constant-product-AMM (Uniswap V2-style) market made of real token symbols (WETH, USDC, USDT, WBTC, DAI, plus long-tail spokes) spread across four venues (Uniswap V2, SushiSwap, PancakeSwap, Biswap) with per-venue fees. Every swap is priced exactly — U256 reserves, Uniswap V2 formula, wei-accurate outputs.

Three tabs:

- **Router** — pick any src / dst / amount; compare routing algorithms on the same trade. Live per-leg breakdown, allocation bars, and a "vs best direct pool" delta.
- **Arbitrage** — scan for profitable cycles starting and ending in a chosen token. Inject a perturbation on any pool to manufacture one on demand, then replay it with exact U256 profit.
- **Benchmarks** — native-Rust timing for each algorithm across graph sizes (10 → 1000 tokens), plus an "average % improvement vs best direct swap" column that actually measures routing quality, not just speed.

## Algorithms

All implementations live in [`core/src/algo`](core/src/algo) and share a common `SolveResult` / `Outcome` vocabulary plus a shared test suite (12 conformance invariants + per-algorithm tests).

| Algorithm | Tab | Handles | Notes |
|---|---|---|---|
| **Dijkstra** | Router | non-negative edges only | Textbook log-weighted shortest path. Slippage-blind. |
| **Bellman-Ford** | Router + Arbitrage | negative edges, detects negative cycles | V-1 relaxation passes. Surfaces the cycle when one exists. |
| **Amount-aware** | Router | slippage, parallel pools | Top-K bounded-hop BF shortlist rescored by exact U256 `simulate_path`. Equivalent to what 1inch's Pathfinder does: linear in edges per hop, correct on negative edges and cycles. |
| **Split (DP)** | Router | slippage, split routing | Uniswap SOR-style knapsack over discrete chunks (10%) across top-K candidate paths. Reports realistic output via sequential simulation. |
| **Split (Frank-Wolfe)** | Router | slippage, split routing, shared-pool interference | Iterative convex optimiser: each iteration recomputes pool reserves from committed flow, runs shortest-path on the updated marginal-price graph, line-searches the step size. Converges to a realistic optimum on the simplex. |
| **Targeted arb scan** | Arbitrage | arbitrage cycles through a specific entry token | DFS-enumerates simple cycles of length ≤ 4 that start and end in the chosen token, scores by exact U256 profit. |

## Architecture notes

**Two-layer numerics.** Routing in `f64` log-space (cheap, marginal rates only); execution in `U256` with Uniswap's exact formula (wei-accurate). Every `Outcome::Found` carries both so the UI can show them side by side.

**Honest reporting.** Split routers report `amount_out` via sequential simulation of the chosen allocation — what would actually execute atomically on-chain. DP's internal knapsack still uses an optimistic per-route quote (matching Uniswap SOR), but the reported number is realistic.

**Trace is optional.** `SolveOptions { with_trace }` toggles step emission. The frontend keeps it on for animation; benchmarks turn it off, which is a meaningful speedup on Dijkstra / BF at V ≥ 300.

## Workspace layout

```
routeviz/
├── core/          # Pure Rust: tokens, pools, graph, generator, algorithms, testkit
├── wasm/          # wasm-bindgen layer — generate_graph, solve, scan_arb, inject_arb, relayout
├── cli/           # Two binaries:
│                  #   `routeviz` — info + solve from the terminal
│                  #   `bench`    — benchmark matrix → JSON report
└── web/           # Vite + React frontend (consumes wasm/pkg)
```

## Development

### Prerequisites

- Rust stable (1.80+) with the `wasm32-unknown-unknown` target
- [`wasm-pack`](https://rustwasm.github.io/wasm-pack/installer/)
- Node 20+ and npm

### Build + run

```bash
# 1. Build the WASM bindings
cd wasm
wasm-pack build --target web --release
cd ..

# 2. Install web deps and start Vite
cd web
npm install
npm run dev
# open http://localhost:5173/
```

The Vite dev server serves from `/` locally and from `/routeviz/` in production — handled in `web/vite.config.ts`.

If you rebuild `wasm-pack` while Vite is already running, restart it with `npm run dev -- --force` to clear the pre-bundle cache.

### Tests

```bash
cargo test --workspace              # 125 tests across core
cd web && npx tsc -b --noEmit       # TypeScript type-check
```

### Benchmarks

The bench binary runs every algorithm against 5 graph sizes (10 / 30 / 100 / 300 / 1000 tokens) with a fixed seed, 250 samples per config, and writes a JSON report the **Benchmarks** tab renders as grouped bar charts + a "vs direct" improvement column.

```bash
cargo run --release -p routeviz-cli --bin bench -- --out web/public/benchmarks.json
```

Bench runs with `with_trace: false` so timings reflect algorithmic work only. Numbers are native-Rust; browser runs are slightly slower due to WASM overhead and the JS↔Rust boundary, as the Benchmarks tab notes.

### CLI

```bash
# Solve a default-graph query with amount-aware
cargo run --release -p routeviz-cli --bin routeviz -- solve \
  --algo amount-aware --from WETH --to USDC --amount 1

# Same graph, split routing across top-K paths
cargo run --release -p routeviz-cli --bin routeviz -- solve \
  --algo split-fw --from WETH --to USDC --amount 10

# Print the generator's graph summary
cargo run --release -p routeviz-cli --bin routeviz -- info
```

## Deployment (GitHub Pages)

CI runs on every push; the **Deploy** workflow publishes to Pages on every merge to `main`.

One-time repo setup:

1. Push the repo to GitHub.
2. Settings → Pages → **Source: GitHub Actions**.
3. Merge something to `main` (or run the `Deploy` workflow manually from the Actions tab).

The deploy workflow:
- installs the Rust toolchain + wasm-pack
- builds the WASM bindings
- runs the benchmark binary, writing fresh results into `web/public/benchmarks.json`
- `npm ci && npm run build` in `web/`
- uploads `web/dist` as the Pages artifact and deploys

## Implementation details

- **Trace vocabulary**: `Step::Visit(u)`, `Step::Relax { from, to, new_distance }`, `Step::Pass(n)` (Bellman-Ford only). On in the WASM binding, off in benchmarks.
- **Candidate generator**: `BoundedBfIter` is a top-K bounded-hop BF shared by amount-aware, Split-DP and Split-FW. Consumers treat it as a path enumerator and rerank by realised output net of gas — log-weight is slippage-blind.
- **Graph shape**: hub-and-spoke topology — 5 hubs always fully connected, spokes connect to hubs at a configurable pair density. Parallel pools come from multiple venues (10 / 25 / 30 bps depending on venue).
- **Negative cycles**: Bellman-Ford returns `Outcome::NegativeCycle { cycle, pools_used, cycle_output, .. }`. The Arbitrage tab replays it with exact U256 math.

## License

MIT. See [LICENSE](LICENSE).
