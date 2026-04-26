import { useCallback, useEffect, useMemo, useState } from "react";
import { TokenGraphCanvas } from "./TokenGraphCanvas";
import { GraphPanel } from "./GraphPanel";
import { PlaybackControls } from "./PlaybackControls";
import type {
  Address,
  GenConfig,
  GeneratedGraph,
  LayoutMode,
  Pool,
  SolveResult,
} from "./types";
import { formatUnits, shortAddress } from "./types";
import type {
  InjectArbRequest,
  ScanArbRequest,
} from "./useRouteviz";

interface Props {
  graph: GeneratedGraph | null;
  config: GenConfig;
  lockSeed: boolean;
  setLockSeed: (v: boolean) => void;
  updateConfig: (patch: Partial<GenConfig>) => void;
  regenerate: () => void;
  solve: (req: import("./useRouteviz").SolveRequest) => SolveResult;
  injectArb: (req: InjectArbRequest) => Pool[];
  scanArb: (req: ScanArbRequest) => SolveResult;
  onUpdatePools: (pools: Pool[]) => void;
  layoutMode: LayoutMode;
  changeLayout: (m: LayoutMode) => void;
  gasPriceGwei: number;
  setGasPriceGwei: (v: number) => void;
  logicalWidth: number;
  logicalHeight: number;
}

const randSeed = () => BigInt(Math.floor(Math.random() * 0x7fffffff));

export function ArbView({
  graph,
  config,
  lockSeed,
  setLockSeed,
  updateConfig,
  regenerate,
  scanArb,
  injectArb,
  onUpdatePools,
  layoutMode,
  changeLayout,
  gasPriceGwei,
  setGasPriceGwei,
  logicalWidth,
  logicalHeight,
}: Props) {
  const [entry, setEntry] = useState<Address | null>(null);
  const [magnitude, setMagnitude] = useState(0.05);
  const [result, setResult] = useState<SolveResult | null>(null);
  const [animIndex, setAnimIndex] = useState(0);
  const [playing, setPlaying] = useState(false);
  const [speed, setSpeed] = useState(60);
  const [solveMs, setSolveMs] = useState<number | null>(null);

  // Default entry: WETH > first hub > first token.
  useEffect(() => {
    if (!graph || graph.tokens.length === 0) {
      setEntry(null);
      return;
    }
    const weth = graph.tokens.find((t) => t.symbol === "WETH");
    const firstHub = graph.tokens.find((t) => t.kind === "Hub");
    setEntry((weth ?? firstHub ?? graph.tokens[0]).address);
  }, [graph]);

  const entryToken = useMemo(
    () => graph?.tokens.find((t) => t.address === entry) ?? null,
    [graph, entry],
  );

  const clearRun = useCallback(() => {
    setResult(null);
    setAnimIndex(0);
    setPlaying(false);
    setSolveMs(null);
  }, []);

  // Canvas click → set entry token.
  const handleTokenClick = useCallback(
    (addr: Address) => {
      if (addr === entry) return;
      setEntry(addr);
      clearRun();
    },
    [entry, clearRun],
  );

  const runScan = useCallback(
    (overrideGraph?: GeneratedGraph) => {
      const g = overrideGraph ?? graph;
      if (!g || !entry) return;
      const t0 = performance.now();
      const r = scanArb({
        tokens: g.tokens,
        pools: g.pools,
        entry,
        gas_price_gwei: gasPriceGwei,
      });
      setSolveMs(performance.now() - t0);
      setResult(r);
      setAnimIndex(0);
      // Skip auto-play on NoPath — the DFS exhaustion trace looks stuck.
      const foundCycle =
        typeof r.outcome === "object" && "NegativeCycle" in r.outcome;
      setPlaying(foundCycle);
    },
    [graph, entry, scanArb, gasPriceGwei],
  );

  const handleInjectArb = useCallback(() => {
    if (!graph) return;
    const newPools = injectArb({
      pools: graph.pools,
      magnitude,
      seed: randSeed(),
    });
    onUpdatePools(newPools);
    runScan({ ...graph, pools: newPools });
  }, [graph, magnitude, injectArb, onUpdatePools, runScan]);

  const outcomeCycle =
    result && typeof result.outcome === "object" && "NegativeCycle" in result.outcome
      ? result.outcome.NegativeCycle
      : null;
  const outcomeNoPath =
    !!result && typeof result.outcome === "string" && result.outcome === "NoPath";

  const showSolution =
    !!result && animIndex >= (result?.trace.length ?? 0);

  const cycleEntryToken = useMemo(() => {
    if (!outcomeCycle || !graph) return null;
    return graph.tokens.find((t) => t.address === outcomeCycle.cycle[0]) ?? null;
  }, [outcomeCycle, graph]);

  // Core's ternary search already picked the profit-maximising amount;
  // we just translate the reported numbers for display.
  const cycleStats = useMemo(() => {
    if (!outcomeCycle || !cycleEntryToken) return null;
    const amountIn = BigInt(outcomeCycle.amount_in);
    const cycleOutput = BigInt(outcomeCycle.cycle_output);
    const gasCost = BigInt(outcomeCycle.gas_cost);
    if (amountIn === 0n) return null;
    const grossProfit =
      cycleOutput > amountIn ? cycleOutput - amountIn : 0n;
    const netProfit = grossProfit > gasCost ? grossProfit - gasCost : 0n;
    const grossScaled = (grossProfit * 100_000n) / amountIn;
    const grossPct = Number(grossScaled) / 1000;
    const netScaled = (netProfit * 100_000n) / amountIn;
    const netPct = Number(netScaled) / 1000;
    return {
      amountIn,
      cycleOutput,
      gasCost,
      grossProfit,
      netProfit,
      grossPct,
      netPct,
    };
  }, [outcomeCycle, cycleEntryToken]);

  return (
    <div className="app-body">
      <aside className="sidebar">
        <GraphPanel
          config={config}
          lockSeed={lockSeed}
          setLockSeed={setLockSeed}
          updateConfig={updateConfig}
          regenerate={regenerate}
          layoutMode={layoutMode}
          changeLayout={changeLayout}
          gasPriceGwei={gasPriceGwei}
          setGasPriceGwei={setGasPriceGwei}
          showVenuesHint
          onBeforeRegenerate={clearRun}
        />

        <section className="panel">
          <h3>Scan</h3>
          <p className="hint">
            Find a profitable cycle that starts and ends in the token you
            hold — matches how an arbitrageur actually trades (put in, loop,
            get more out, same token).
          </p>
          <label>
            <span>Hold / return to</span>
            <select
              value={entry ?? ""}
              onChange={(e) => {
                setEntry(e.target.value);
                clearRun();
              }}
            >
              {graph?.tokens.map((t) => (
                <option key={t.address} value={t.address}>
                  {t.symbol}
                  {t.kind === "Hub" ? " ★" : ""}
                </option>
              ))}
            </select>
          </label>
          <p className="hint">
            Trade size is solved per-candidate-cycle: each cycle's
            profit-vs-input-amount is a concave hump, and the scanner
            ternary-searches for the size that maximises end-to-end
            profit (the marginal rate around the cycle drops to 1).
          </p>
          <button
            className="btn btn-primary btn-full"
            onClick={() => runScan()}
            disabled={!graph || !entry}
          >
            🔍 Scan
          </button>

          <div className="panel-sep" />
          <label>
            <span>Inject magnitude</span>
            <span className="mono small">
              {(magnitude * 100).toFixed(1)}%
            </span>
          </label>
          <input
            type="range"
            min={0.01}
            max={0.2}
            step={0.005}
            value={magnitude}
            onChange={(e) => setMagnitude(+e.target.value)}
          />
          <button
            className="btn btn-danger btn-full"
            onClick={handleInjectArb}
            disabled={!graph}
          >
            💉 Inject arb
          </button>

          {result && (
            <>
              <div className="panel-sep" />
              <PlaybackControls
                result={result}
                animIndex={animIndex}
                setAnimIndex={setAnimIndex}
                playing={playing}
                setPlaying={setPlaying}
                speed={speed}
                setSpeed={setSpeed}
              />
            </>
          )}
        </section>

        <section className="panel stats">
          <h3>Result</h3>
          {result && !outcomeCycle && outcomeNoPath && entryToken && (
            <div className="warn-box">
              <b>No arbitrage cycle found</b>
              <p className="hint">
                No profitable loop through{" "}
                <b className="mono">{entryToken.symbol}</b> at any trade
                size.{" "}
                {config.price_noise === 0
                  ? "This graph is strictly arb-free — "
                  : ""}
                Click <b>💉 Inject arb</b> to perturb a pool, bump{" "}
                <b>Price noise</b> to sprinkle small arbs across the
                graph, or pick a different entry token.
              </p>
            </div>
          )}
          <div className="stat">
            <span>Tokens / Pools</span>
            <b>
              {graph?.tokens.length ?? 0} / {graph?.pools.length ?? 0}
            </b>
          </div>
          <div className="stat">
            <span>Solve time</span>
            <b>{solveMs !== null ? `${solveMs.toFixed(2)} ms` : "—"}</b>
          </div>
          <div className="stat">
            <span>Trace</span>
            <b>{result ? `${animIndex}/${result.trace.length}` : "—"}</b>
          </div>

          {outcomeCycle && cycleEntryToken && cycleStats && (
            <>
              <div className="stat">
                <span>Cycle length</span>
                <b>{outcomeCycle.pools_used.length} hops</b>
              </div>
              <div className="stat">
                <span>Entry token</span>
                <b>{cycleEntryToken.symbol}</b>
              </div>
              <div className="stat">
                <span>Optimal input</span>
                <b>
                  {formatUnits(outcomeCycle.amount_in, cycleEntryToken.decimals)}{" "}
                  {cycleEntryToken.symbol}
                </b>
              </div>
              <div className="stat">
                <span>Cycle output</span>
                <b className="mono">
                  {formatUnits(
                    outcomeCycle.cycle_output,
                    cycleEntryToken.decimals,
                    8,
                  )}{" "}
                  {cycleEntryToken.symbol}
                </b>
              </div>
              {gasPriceGwei > 0 ? (
                <>
                  <div className="stat">
                    <span>Gas cost</span>
                    <b className="mono delta-neg">
                      −
                      {formatUnits(
                        outcomeCycle.gas_cost,
                        cycleEntryToken.decimals,
                        8,
                      )}{" "}
                      {cycleEntryToken.symbol}
                    </b>
                  </div>
                  <div className="stat">
                    <span>Gross profit</span>
                    <b className="mono delta-pos">
                      +{cycleStats.grossPct.toFixed(3)}%
                    </b>
                  </div>
                  <div className="stat">
                    <span>Net profit</span>
                    <b
                      className={
                        cycleStats.netPct > 0
                          ? "mono delta-pos"
                          : "mono delta-neg"
                      }
                    >
                      {cycleStats.netPct > 0 ? "+" : ""}
                      {cycleStats.netPct.toFixed(3)}%
                    </b>
                  </div>
                </>
              ) : (
                <div className="stat">
                  <span>Exact profit</span>
                  <b
                    className={
                      cycleStats.grossPct > 0
                        ? "mono delta-pos"
                        : "mono delta-neg"
                    }
                  >
                    {cycleStats.grossPct > 0 ? "+" : ""}
                    {cycleStats.grossPct.toFixed(3)}%
                  </b>
                </div>
              )}
              <div className="path-summary">
                <span className="path-label">Cycle</span>
                <div className="path-chips cycle-chips">
                  {outcomeCycle.cycle.map((addr, i) => {
                    const t = graph?.tokens.find((tok) => tok.address === addr);
                    return (
                      <span key={`${addr}-${i}`} className="path-chip cycle-chip">
                        {t?.symbol ?? shortAddress(addr)}
                      </span>
                    );
                  })}
                  <span className="path-chip cycle-chip">
                    {cycleEntryToken.symbol}
                  </span>
                </div>
              </div>
            </>
          )}

        </section>
      </aside>

      <main className="viz">
        <TokenGraphCanvas
          graph={graph}
          result={result}
          animIndex={animIndex}
          showPath={showSolution}
          src={entry}
          dst={null}
          onTokenClick={handleTokenClick}
          cycleStyle="profit"
          logicalWidth={logicalWidth}
          logicalHeight={logicalHeight}
        />
      </main>
    </div>
  );
}
