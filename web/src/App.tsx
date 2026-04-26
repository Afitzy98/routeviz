import { useCallback, useEffect, useRef, useState } from "react";
import { useRouteviz } from "./useRouteviz";
import { RouterView } from "./RouterView";
import { ArbView } from "./ArbView";
import { BenchmarksView } from "./BenchmarksView";
import type { GenConfig, GeneratedGraph, LayoutMode, Pool } from "./types";

// Logical canvas size; hub-spoke layout uses radii 140 and 420.
const GRAPH_W = 1000;
const GRAPH_H = 1000;

const randSeed = () => BigInt(Math.floor(Math.random() * 0x7fffffff));

type Tab = "router" | "arbitrage" | "benchmarks";

const TABS: { id: Tab; label: string; available: boolean }[] = [
  { id: "router", label: "Router", available: true },
  { id: "arbitrage", label: "Arbitrage", available: true },
  { id: "benchmarks", label: "Benchmarks", available: true },
];

export default function App() {
  const { ready, generateGraph, defaultConfig, solve, injectArb, scanArb, relayout } =
    useRouteviz();

  const [tab, setTab] = useState<Tab>("router");
  const [config, setConfig] = useState<GenConfig | null>(null);
  const [lockSeed, setLockSeed] = useState(false);
  const [graph, setGraph] = useState<GeneratedGraph | null>(null);
  const [layoutMode, setLayoutMode] = useState<LayoutMode>("hub_spoke");
  // 1 gwei = typical L2 pricing. 0 disables gas accounting.
  const [gasPriceGwei, setGasPriceGwei] = useState(1);
  const didInitRef = useRef(false);

  useEffect(() => {
    if (!ready || didInitRef.current) return;
    didInitRef.current = true;
    const c = defaultConfig();
    setConfig(c);
    setGraph(generateGraph(c));
  }, [ready, defaultConfig, generateGraph]);

  const regenerate = useCallback(() => {
    if (!config) return;
    const next = lockSeed ? config : { ...config, seed: randSeed() };
    setConfig(next);
    let g = generateGraph(next);
    if (layoutMode === "force_directed") {
      const positions = relayout({
        tokens: g.tokens,
        pools: g.pools,
        layout: "force_directed",
        seed: next.seed,
      });
      g = { ...g, positions };
    }
    setGraph(g);
  }, [config, lockSeed, generateGraph, layoutMode, relayout]);

  const updateConfig = useCallback((patch: Partial<GenConfig>) => {
    setConfig((prev) => (prev ? { ...prev, ...patch } : prev));
  }, []);

  // ArbView's inject_arb path: keep seed + positions, swap pools.
  const updatePools = useCallback((pools: Pool[]) => {
    setGraph((prev) => (prev ? { ...prev, pools } : prev));
  }, []);

  // Recompute positions only; tokens/pools unchanged.
  const changeLayout = useCallback(
    (next: LayoutMode) => {
      if (next === layoutMode || !graph) {
        setLayoutMode(next);
        return;
      }
      const positions = relayout({
        tokens: graph.tokens,
        pools: graph.pools,
        layout: next,
        seed: graph.config.seed,
      });
      setGraph({ ...graph, positions });
      setLayoutMode(next);
    },
    [graph, layoutMode, relayout],
  );

  if (!ready || !config) {
    return (
      <div className="app-loading">
        <div className="spinner" />
        <p>Loading WASM…</p>
      </div>
    );
  }

  return (
    <div className="app">
      <div className="app-header">
        <div className="brand">
          <h1>RouteViz</h1>
          <p>dex routing &amp; arbitrage</p>
        </div>
        <div className="app-header-right">
          <div className="tabs top-tabs">
            {TABS.map((t) => (
              <button
                key={t.id}
                className={tab === t.id ? "active" : ""}
                onClick={() => t.available && setTab(t.id)}
                disabled={!t.available}
                title={t.available ? undefined : "coming soon"}
              >
                {t.label}
                {!t.available ? " (soon)" : ""}
              </button>
            ))}
          </div>
          <a
            className="repo-link"
            href="https://github.com/Afitzy98/routeviz"
            target="_blank"
            rel="noopener noreferrer"
            title="View source on GitHub"
          >
            <svg viewBox="0 0 16 16" width="18" height="18" aria-hidden="true">
              <path
                fill="currentColor"
                d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.013 8.013 0 0016 8c0-4.42-3.58-8-8-8z"
              />
            </svg>
            <span>GitHub</span>
          </a>
        </div>
      </div>

      {tab === "router" && (
        <RouterView
          graph={graph}
          config={config}
          lockSeed={lockSeed}
          setLockSeed={setLockSeed}
          updateConfig={updateConfig}
          regenerate={regenerate}
          solve={solve}
          layoutMode={layoutMode}
          changeLayout={changeLayout}
          gasPriceGwei={gasPriceGwei}
          setGasPriceGwei={setGasPriceGwei}
          logicalWidth={GRAPH_W}
          logicalHeight={GRAPH_H}
        />
      )}
      {tab === "arbitrage" && (
        <ArbView
          graph={graph}
          config={config}
          lockSeed={lockSeed}
          setLockSeed={setLockSeed}
          updateConfig={updateConfig}
          regenerate={regenerate}
          solve={solve}
          injectArb={injectArb}
          scanArb={scanArb}
          onUpdatePools={updatePools}
          layoutMode={layoutMode}
          changeLayout={changeLayout}
          gasPriceGwei={gasPriceGwei}
          setGasPriceGwei={setGasPriceGwei}
          logicalWidth={GRAPH_W}
          logicalHeight={GRAPH_H}
        />
      )}
      {tab === "benchmarks" && <BenchmarksView />}
    </div>
  );
}
