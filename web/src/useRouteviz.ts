import { useCallback, useEffect, useState } from "react";
import init, {
  generate_graph,
  default_config,
  solve as solve_wasm,
  inject_arb as inject_arb_wasm,
  scan_arb as scan_arb_wasm,
  relayout as relayout_wasm,
} from "routeviz-wasm";
import type {
  Address,
  AlgorithmId,
  GenConfig,
  GeneratedGraph,
  LayoutMode,
  Point,
  Pool,
  SolveResult,
  Token,
  U256Hex,
} from "./types";

export interface SolveRequest {
  algo: AlgorithmId;
  tokens: Token[];
  pools: Pool[];
  src: Address;
  dst: Address;
  amount_in: U256Hex;
  /** When false, core skips building the `trace` vec — meaningful
   * speedup on Dijkstra / BF at large graphs. Defaults to true in the
   * WASM binding so callers that want to animate don't need to pass
   * anything. */
  with_trace?: boolean;
  /** Gas price in gwei. Zero (or omitted) disables gas accounting —
   * algorithms optimise on gross output and `Outcome.gas_cost = 0`.
   * With a non-zero value, algorithms include gas in their internal
   * objective and report `gas_cost` for UI net-output rendering. */
  gas_price_gwei?: number;
}

export interface InjectArbRequest {
  pools: Pool[];
  magnitude: number;
  seed: bigint;
}

export interface ScanArbRequest {
  tokens: Token[];
  pools: Pool[];
  entry: Address;
  /** Gas price in gwei. Zero (or omitted) disables gas accounting in
   * the per-cycle profit search; with a non-zero value, cycles whose
   * optimal profit doesn't clear gas return as NoPath. */
  gas_price_gwei?: number;
}

export interface RelayoutRequest {
  tokens: Token[];
  pools: Pool[];
  layout: LayoutMode;
  seed: bigint;
}

export function useRouteviz() {
  const [ready, setReady] = useState(false);

  useEffect(() => {
    init().then(() => setReady(true));
  }, []);

  const generateGraph = useCallback(
    (config: GenConfig): GeneratedGraph =>
      generate_graph(config) as GeneratedGraph,
    [],
  );

  const defaultConfig = useCallback(
    (): GenConfig => default_config() as GenConfig,
    [],
  );

  const solve = useCallback(
    (request: SolveRequest): SolveResult => solve_wasm(request) as SolveResult,
    [],
  );

  const injectArb = useCallback(
    (request: InjectArbRequest): Pool[] =>
      inject_arb_wasm(request) as Pool[],
    [],
  );

  const scanArb = useCallback(
    (request: ScanArbRequest): SolveResult =>
      scan_arb_wasm(request) as SolveResult,
    [],
  );

  const relayout = useCallback(
    (request: RelayoutRequest): Point[] =>
      relayout_wasm(request) as Point[],
    [],
  );

  return { ready, generateGraph, defaultConfig, solve, injectArb, scanArb, relayout };
}
