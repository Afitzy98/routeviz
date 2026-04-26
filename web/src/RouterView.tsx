import { useCallback, useEffect, useMemo, useState } from "react";
import { TokenGraphCanvas } from "./TokenGraphCanvas";
import { GraphPanel } from "./GraphPanel";
import { PlaybackControls } from "./PlaybackControls";
import type {
  Address,
  AlgorithmId,
  GenConfig,
  GeneratedGraph,
  LayoutMode,
  SolveResult,
} from "./types";
import {
  ALGORITHMS,
  formatUnits,
  parseUnits,
  shortAddress,
  usdToBaseUnits,
} from "./types";
import type { SolveRequest } from "./useRouteviz";

interface Props {
  graph: GeneratedGraph | null;
  config: GenConfig;
  lockSeed: boolean;
  setLockSeed: (v: boolean) => void;
  updateConfig: (patch: Partial<GenConfig>) => void;
  regenerate: () => void;
  solve: (req: SolveRequest) => SolveResult;
  layoutMode: LayoutMode;
  changeLayout: (m: LayoutMode) => void;
  gasPriceGwei: number;
  setGasPriceGwei: (v: number) => void;
  logicalWidth: number;
  logicalHeight: number;
}

export function RouterView({
  graph,
  config,
  lockSeed,
  setLockSeed,
  updateConfig,
  regenerate,
  solve,
  layoutMode,
  changeLayout,
  gasPriceGwei,
  setGasPriceGwei,
  logicalWidth,
  logicalHeight,
}: Props) {
  const [algorithm, setAlgorithm] = useState<AlgorithmId>("dijkstra");
  const [src, setSrc] = useState<Address | null>(null);
  const [dst, setDst] = useState<Address | null>(null);
  const [amountInHuman, setAmountInHuman] = useState<string>("1000");
  const [amountMode, setAmountMode] = useState<"usd" | "token">("usd");
  const [result, setResult] = useState<SolveResult | null>(null);
  const [animIndex, setAnimIndex] = useState(0);
  const [playing, setPlaying] = useState(false);
  const [speed, setSpeed] = useState(30);
  const [solveMs, setSolveMs] = useState<number | null>(null);
  const [pickMode, setPickMode] = useState<"src" | "dst">("src");

  // On graph change, default to first two hubs.
  useEffect(() => {
    if (!graph) return;
    const hubs = graph.tokens.filter((t) => t.kind === "Hub");
    if (hubs.length >= 2) {
      setSrc(hubs[0].address);
      setDst(hubs[1].address);
    } else if (graph.tokens.length >= 2) {
      setSrc(graph.tokens[0].address);
      setDst(graph.tokens[1].address);
    } else {
      setSrc(null);
      setDst(null);
    }
    setResult(null);
    setAnimIndex(0);
    setPlaying(false);
    setSolveMs(null);
    setPickMode("src");
  }, [graph]);

  const srcToken = useMemo(
    () => graph?.tokens.find((t) => t.address === src) ?? null,
    [graph, src],
  );
  const dstToken = useMemo(
    () => graph?.tokens.find((t) => t.address === dst) ?? null,
    [graph, dst],
  );

  const clearRun = useCallback(() => {
    setResult(null);
    setAnimIndex(0);
    setPlaying(false);
    setSolveMs(null);
  }, []);

  // Amount → U256 hex. "usd" divides by src.true_price_usd first.
  const amountInHex = useMemo<string>(() => {
    if (!srcToken) return "0x0";
    if (amountMode === "token") {
      return parseUnits(amountInHuman, srcToken.decimals);
    }
    const usd = parseFloat(amountInHuman);
    const wei = usdToBaseUnits(usd, srcToken.true_price_usd, srcToken.decimals);
    return wei === null ? "0x0" : "0x" + wei.toString(16);
  }, [amountInHuman, amountMode, srcToken]);

  // "Other unit" hint shown beneath the amount input.
  const amountAltDisplay = useMemo<string>(() => {
    if (!srcToken) return "";
    const value = parseFloat(amountInHuman);
    if (!Number.isFinite(value) || value <= 0) return "";
    if (amountMode === "usd") {
      const tokens = value / srcToken.true_price_usd;
      const decimals = tokens >= 1 ? 4 : 6;
      return `≈ ${tokens.toFixed(decimals)} ${srcToken.symbol}`;
    }
    const usd = value * srcToken.true_price_usd;
    return `≈ $${usd.toLocaleString(undefined, { maximumFractionDigits: 2 })}`;
  }, [amountInHuman, amountMode, srcToken]);

  const runSolve = useCallback(() => {
    if (!graph || !src || !dst || src === dst || !srcToken) return;
    const t0 = performance.now();
    const r = solve({
      algo: algorithm,
      tokens: graph.tokens,
      pools: graph.pools,
      src,
      dst,
      amount_in: amountInHex,
      gas_price_gwei: gasPriceGwei,
    });
    setSolveMs(performance.now() - t0);
    setResult(r);
    setAnimIndex(0);
    setPlaying(true);
  }, [graph, src, dst, srcToken, amountInHex, algorithm, solve, gasPriceGwei]);

  const handleTokenClick = useCallback(
    (addr: Address, shift: boolean) => {
      const setAsDst = shift || pickMode === "dst";
      if (setAsDst) {
        if (addr !== src) {
          setDst(addr);
          clearRun();
          setPickMode("src");
        }
      } else {
        if (addr !== dst) {
          setSrc(addr);
          clearRun();
          setPickMode("dst");
        }
      }
    },
    [src, dst, pickMode, clearRun],
  );

  const outcomeFound =
    result && typeof result.outcome === "object" && "Found" in result.outcome
      ? result.outcome.Found
      : null;
  const outcomeSplit =
    result && typeof result.outcome === "object" && "FoundSplit" in result.outcome
      ? result.outcome.FoundSplit
      : null;
  const outcomeCycle =
    result &&
    typeof result.outcome === "object" &&
    "NegativeCycle" in result.outcome
      ? result.outcome.NegativeCycle
      : null;
  const outcomeNoPath =
    !!result && typeof result.outcome === "string" && result.outcome === "NoPath";
  // Show solved state once trace animation has finished.
  const showSolution =
    !!result &&
    animIndex >= (result?.trace.length ?? 0) &&
    (!!outcomeFound || !!outcomeSplit || !!outcomeCycle);

  // Best direct A↔B swap output, for the "vs direct" delta. Computed
  // client-side with the same constant-product math as core.
  const directComparison = useMemo(() => {
    if (!graph || !src || !dst || src === dst || !srcToken || !dstToken)
      return null;
    const optimalGrossHex =
      outcomeFound?.amount_out ?? outcomeSplit?.amount_out;
    const optimalGasHex =
      outcomeFound?.gas_cost ?? outcomeSplit?.gas_cost ?? "0x0";
    if (!optimalGrossHex) return null;
    const directs = graph.pools.filter(
      (p) =>
        (p.token_a === src && p.token_b === dst) ||
        (p.token_a === dst && p.token_b === src),
    );
    if (directs.length === 0) return null;
    const amountIn = BigInt(amountInHex);
    if (amountIn === 0n) return null;
    let bestOut = 0n;
    let bestPool: string | null = null;
    for (const pool of directs) {
      const [rIn, rOut] =
        pool.token_a === src
          ? [BigInt(pool.reserve_a), BigInt(pool.reserve_b)]
          : [BigInt(pool.reserve_b), BigInt(pool.reserve_a)];
      if (rIn === 0n || rOut === 0n) continue;
      const feeMult = 10000n - BigInt(pool.fee_bps);
      const amountInWithFee = amountIn * feeMult;
      const numerator = amountInWithFee * rOut;
      const denominator = rIn * 10000n + amountInWithFee;
      if (denominator === 0n) continue;
      const out = numerator / denominator;
      if (out > bestOut) {
        bestOut = out;
        bestPool = pool.address;
      }
    }
    if (bestOut === 0n || !bestPool) return null;

    // Constants must match core/src/algo/gas.rs.
    const SINGLE_SWAP_GAS = 21_000n + 30_000n + 90_000n; // 141k
    const ETH_PRICE_USD = 3000n;
    const gweiBig = BigInt(Math.round(gasPriceGwei * 1000));
    const gasUsdF =
      Number(SINGLE_SWAP_GAS) * gasPriceGwei * 1e-9 * Number(ETH_PRICE_USD);
    let directGas = 0n;
    if (dstToken && gasUsdF > 0) {
      const dstHuman = gasUsdF / dstToken.true_price_usd;
      const scale = 10n ** BigInt(dstToken.decimals);
      const whole = BigInt(Math.floor(dstHuman));
      const fracF = dstHuman - Math.floor(dstHuman);
      const fracScaled = BigInt(Math.floor(fracF * Number(scale)));
      directGas = whole * scale + fracScaled;
    }
    void gweiBig; // touched only for type stability

    const directNet = bestOut > directGas ? bestOut - directGas : 0n;
    const optimalGross = BigInt(optimalGrossHex);
    const optimalGas = BigInt(optimalGasHex);
    const optimalNet = optimalGross > optimalGas ? optimalGross - optimalGas : 0n;

    // Net vs net so the % stays honest under gas.
    const baseline = directNet === 0n ? bestOut : directNet;
    if (baseline === 0n) return null;
    const scaled = ((optimalNet - directNet) * 100_000n) / baseline;
    const deltaPct = Number(scaled) / 1000;
    const solvedPathIsDirect =
      !!outcomeFound &&
      outcomeFound.pools_used.length === 1 &&
      outcomeFound.pools_used[0] === bestPool;
    return {
      amountOutHex: "0x" + bestOut.toString(16),
      directNetHex: "0x" + directNet.toString(16),
      directGasHex: "0x" + directGas.toString(16),
      poolAddress: bestPool,
      deltaPct,
      solvedPathIsDirect,
    };
  }, [
    graph,
    src,
    dst,
    srcToken,
    dstToken,
    amountInHex,
    outcomeFound,
    outcomeSplit,
    gasPriceGwei,
  ]);

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
        />

        <section className="panel">
          <h3>Route</h3>
          <label>
            <span>Algorithm</span>
            <select
              value={algorithm}
              onChange={(e) => {
                setAlgorithm(e.target.value as AlgorithmId);
                clearRun();
              }}
            >
              {ALGORITHMS.map((a) => (
                <option key={a.id} value={a.id} disabled={!a.available}>
                  {a.name}
                  {a.available ? "" : " (soon)"}
                </option>
              ))}
            </select>
          </label>
          <p className="hint">
            {ALGORITHMS.find((a) => a.id === algorithm)?.blurb}
          </p>
          <label>
            <span className="dot dot-src" />
            <span className="endpoint-label">From</span>
            <select
              value={src ?? ""}
              onChange={(e) => {
                setSrc(e.target.value);
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
          <label>
            <span className="dot dot-dst" />
            <span className="endpoint-label">To</span>
            <select
              value={dst ?? ""}
              onChange={(e) => {
                setDst(e.target.value);
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
          <label>
            <span>Amount in</span>
            <input
              type="text"
              value={amountInHuman}
              onChange={(e) => {
                setAmountInHuman(e.target.value);
                clearRun();
              }}
            />
          </label>
          <div className="pick-mode">
            <span className="hint">Denominated in:</span>
            <div className="toggle">
              <button
                className={amountMode === "usd" ? "active" : ""}
                onClick={() => {
                  setAmountMode("usd");
                  clearRun();
                }}
              >
                USD
              </button>
              <button
                className={amountMode === "token" ? "active" : ""}
                onClick={() => {
                  setAmountMode("token");
                  clearRun();
                }}
              >
                {srcToken?.symbol ?? "Token"}
              </button>
            </div>
          </div>
          {amountAltDisplay && (
            <p className="hint mono small">{amountAltDisplay}</p>
          )}
          <div className="pick-mode">
            <span className="hint">Canvas click sets:</span>
            <div className="toggle">
              <button
                className={pickMode === "src" ? "active" : ""}
                onClick={() => setPickMode("src")}
              >
                From
              </button>
              <button
                className={pickMode === "dst" ? "active" : ""}
                onClick={() => setPickMode("dst")}
              >
                To
              </button>
            </div>
          </div>
          <p className="hint">Shift-click always sets To.</p>
          <button
            className="btn btn-primary btn-full"
            onClick={runSolve}
            disabled={!graph || !src || !dst || src === dst}
          >
            ▶ Solve
          </button>
          {result && (
            <PlaybackControls
              result={result}
              animIndex={animIndex}
              setAnimIndex={setAnimIndex}
              playing={playing}
              setPlaying={setPlaying}
              speed={speed}
              setSpeed={setSpeed}
            />
          )}
        </section>

        <section className="panel stats">
          <h3>Stats</h3>
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
          {outcomeFound && (
            <>
              <div className="stat">
                <span>Hops</span>
                <b>{outcomeFound.pools_used.length}</b>
              </div>
              <div className="stat">
                <span>Product of rates</span>
                <b>{outcomeFound.product_of_rates.toExponential(4)}</b>
              </div>
              {srcToken && dstToken && (
                <>
                  <div className="stat">
                    <span>Input</span>
                    <b>
                      {formatUnits(outcomeFound.amount_in, srcToken.decimals)}{" "}
                      {srcToken.symbol}
                    </b>
                  </div>
                  {gasPriceGwei > 0 ? (
                    <>
                      <div className="stat">
                        <span>Gross output</span>
                        <b className="mono">
                          {formatUnits(
                            outcomeFound.amount_out,
                            dstToken.decimals,
                            6,
                          )}{" "}
                          {dstToken.symbol}
                        </b>
                      </div>
                      <div className="stat">
                        <span>Gas cost</span>
                        <b className="mono delta-neg">
                          −
                          {formatUnits(
                            outcomeFound.gas_cost,
                            dstToken.decimals,
                            6,
                          )}{" "}
                          {dstToken.symbol}
                        </b>
                      </div>
                      <div className="stat">
                        <span>Net output</span>
                        <b className="accent">
                          {formatUnits(
                            "0x" +
                              (
                                BigInt(outcomeFound.amount_out) -
                                BigInt(outcomeFound.gas_cost)
                              ).toString(16),
                            dstToken.decimals,
                            6,
                          )}{" "}
                          {dstToken.symbol}
                        </b>
                      </div>
                    </>
                  ) : (
                    <div className="stat">
                      <span>Output</span>
                      <b className="accent">
                        {formatUnits(
                          outcomeFound.amount_out,
                          dstToken.decimals,
                          6,
                        )}{" "}
                        {dstToken.symbol}
                      </b>
                    </div>
                  )}
                </>
              )}
              <div className="path-summary">
                <span className="path-label">Path</span>
                <div className="path-chips">
                  {outcomeFound.path.map((addr, i) => {
                    const t = graph?.tokens.find((tok) => tok.address === addr);
                    return (
                      <span key={`${addr}-${i}`} className="path-chip">
                        {t?.symbol ?? shortAddress(addr)}
                      </span>
                    );
                  })}
                </div>
              </div>
              {directComparison && srcToken && dstToken && (
                <div className="direct-compare">
                  <span className="path-label">
                    {directComparison.solvedPathIsDirect
                      ? "Using direct pool"
                      : "Direct pool comparison"}
                  </span>
                  <div className="stat">
                    <span>Direct output</span>
                    <b className="mono">
                      {formatUnits(
                        directComparison.amountOutHex,
                        dstToken.decimals,
                        6,
                      )}{" "}
                      {dstToken.symbol}
                    </b>
                  </div>
                  {!directComparison.solvedPathIsDirect && (
                    <>
                      <div className="stat">
                        <span>
                          {ALGORITHMS.find((a) => a.id === algorithm)?.name ??
                            algorithm}{" "}
                          vs direct
                        </span>
                        <b
                          className={
                            directComparison.deltaPct > 0
                              ? "mono delta-pos"
                              : directComparison.deltaPct < 0
                                ? "mono delta-neg"
                                : "mono"
                          }
                        >
                          {directComparison.deltaPct > 0 ? "+" : ""}
                          {directComparison.deltaPct.toFixed(3)}%
                        </b>
                      </div>
                      {directComparison.deltaPct < 0 &&
                        algorithm !== "amount_aware" && (
                          <p className="hint slippage-note">
                            {ALGORITHMS.find((a) => a.id === algorithm)?.name ??
                              algorithm}
                            &apos;s pick loses to direct at this amount.
                            Log-weight searches (Dijkstra, Bellman-Ford) rank
                            token-paths by marginal rate at zero input, so
                            {outcomeFound.pools_used.length > 1 ? (
                              <> multi-hop slippage compounds</>
                            ) : (
                              <>{" "}among parallel pools they favour the
                              best-marginal-rate pool, which can be shallower
                              than another pool at the same pair</>
                            )}
                            {" "}— the direct comparison measures realised
                            output instead. Try{" "}
                            <b>Amount-aware</b>, <b>Split (DP)</b>, or{" "}
                            <b>Split (Frank-Wolfe)</b> — they score paths by
                            exact output at your trade size.
                          </p>
                        )}
                    </>
                  )}
                </div>
              )}
            </>
          )}
          {outcomeSplit && srcToken && dstToken && (
            <>
              <div className="stat">
                <span>Legs</span>
                <b>{outcomeSplit.legs.length}</b>
              </div>
              <div className="stat">
                <span>Input</span>
                <b>
                  {formatUnits(outcomeSplit.amount_in, srcToken.decimals)}{" "}
                  {srcToken.symbol}
                </b>
              </div>
              {gasPriceGwei > 0 ? (
                <>
                  <div className="stat">
                    <span>Gross output</span>
                    <b className="mono">
                      {formatUnits(outcomeSplit.amount_out, dstToken.decimals, 6)}{" "}
                      {dstToken.symbol}
                    </b>
                  </div>
                  <div className="stat">
                    <span>Gas cost</span>
                    <b className="mono delta-neg">
                      −
                      {formatUnits(outcomeSplit.gas_cost, dstToken.decimals, 6)}{" "}
                      {dstToken.symbol}
                    </b>
                  </div>
                  <div className="stat">
                    <span>Net output</span>
                    <b className="accent">
                      {formatUnits(
                        "0x" +
                          (
                            BigInt(outcomeSplit.amount_out) -
                            BigInt(outcomeSplit.gas_cost)
                          ).toString(16),
                        dstToken.decimals,
                        6,
                      )}{" "}
                      {dstToken.symbol}
                    </b>
                  </div>
                </>
              ) : (
                <div className="stat">
                  <span>Output</span>
                  <b className="accent">
                    {formatUnits(outcomeSplit.amount_out, dstToken.decimals, 6)}{" "}
                    {dstToken.symbol}
                  </b>
                </div>
              )}
              <div className="path-summary">
                <span className="path-label">Allocation</span>
                {outcomeSplit.legs.map((leg, i) => {
                  const totalIn = BigInt(outcomeSplit.amount_in);
                  const legIn = BigInt(leg.amount_in);
                  // BigInt math — Number() would overflow at 18 decimals.
                  const pct =
                    totalIn === 0n
                      ? 0
                      : Number((legIn * 100_000n) / totalIn) / 1000;
                  return (
                    <div key={i} className="split-leg">
                      <div className="split-leg-header">
                        <span className="mono small">{pct.toFixed(1)}%</span>
                        <span className="mono small">
                          {formatUnits(leg.amount_out, dstToken.decimals, 6)}{" "}
                          {dstToken.symbol}
                        </span>
                      </div>
                      <div className="path-chips">
                        {leg.path.map((addr, j) => {
                          const t = graph?.tokens.find(
                            (tok) => tok.address === addr,
                          );
                          return (
                            <span
                              key={`${addr}-${j}`}
                              className="path-chip"
                            >
                              {t?.symbol ?? shortAddress(addr)}
                            </span>
                          );
                        })}
                      </div>
                      <div
                        className="split-leg-bar"
                        style={{ width: `${pct}%` }}
                      />
                    </div>
                  );
                })}
              </div>
              {directComparison && (
                <div className="direct-compare">
                  <span className="path-label">Direct pool comparison</span>
                  <div className="stat">
                    <span>Direct output</span>
                    <b className="mono">
                      {formatUnits(
                        directComparison.amountOutHex,
                        dstToken.decimals,
                        6,
                      )}{" "}
                      {dstToken.symbol}
                    </b>
                  </div>
                  <div className="stat">
                    <span>Split vs direct</span>
                    <b
                      className={
                        directComparison.deltaPct > 0
                          ? "mono delta-pos"
                          : directComparison.deltaPct < 0
                            ? "mono delta-neg"
                            : "mono"
                      }
                    >
                      {directComparison.deltaPct > 0 ? "+" : ""}
                      {directComparison.deltaPct.toFixed(3)}%
                    </b>
                  </div>
                </div>
              )}
            </>
          )}
          {outcomeNoPath && (
            <p className="warn">No path between selected tokens.</p>
          )}
          {outcomeCycle && (
            <>
              <div className="stat">
                <span>Cycle length</span>
                <b>{outcomeCycle.pools_used.length} hops</b>
              </div>
              <div className="stat">
                <span>Product of rates</span>
                <b>{outcomeCycle.product_of_rates.toFixed(6)}</b>
              </div>
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
                    {graph?.tokens.find(
                      (t) => t.address === outcomeCycle.cycle[0],
                    )?.symbol ?? "↻"}
                  </span>
                </div>
              </div>
              <p className="hint slippage-note">
                A <b>negative cycle</b> was detected on this graph —
                arbitrage exists. Bellman-Ford prioritises cycle-reporting
                over routing by design. Switch to <b>Dijkstra</b> or
                {" "}<b>Amount-aware</b> for this query, or jump to the
                {" "}<b>Arbitrage</b> tab to inspect the cycle with exact
                profit numbers.
              </p>
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
          src={src}
          dst={dst}
          onTokenClick={handleTokenClick}
          logicalWidth={logicalWidth}
          logicalHeight={logicalHeight}
        />
      </main>
    </div>
  );
}
