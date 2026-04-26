import { useEffect, useState } from "react";
import { ALGORITHMS } from "./types";

interface BenchRow {
  algorithm: string;
  num_tokens: number;
  num_pools: number;
  samples: number;
  median_ms: number;
  min_ms: number;
  max_ms: number;
  /// Avg % of algo net output vs best direct pool. Null if disabled.
  improvement_pct: number | null;
  /// Per-pair % improvements; the average above is the mean of these.
  improvement_ratios?: number[];
  /// Raw per-sample timings in ms.
  times_ms?: number[];
}

// Per-size range so all algos at one n share an x-axis.
interface SizeRange {
  min: number;
  max: number;
}

const HIST_BINS = 18;
const HIST_W = 200;
const HIST_H = 36;

interface BenchReport {
  generated_at_unix: number;
  seed: number;
  pair_density: number;
  sizes: number[];
  samples_per_config: number;
  results: BenchRow[];
}

function algoInfo(id: string): { name: string; color: string } {
  const hit = ALGORITHMS.find((a) => a.id === id);
  return { name: hit?.name ?? id, color: hit?.color ?? "#888" };
}

function formatDuration(ms: number): { value: string; unit: string } {
  if (ms < 1) return { value: (ms * 1000).toFixed(2), unit: "µs" };
  if (ms < 1000) return { value: ms.toFixed(2), unit: "ms" };
  return { value: (ms / 1000).toFixed(2), unit: "s" };
}

type LoadState =
  | { kind: "loading" }
  | { kind: "ready"; report: BenchReport }
  | { kind: "missing" }
  | { kind: "error"; message: string };

// Per-row range so a single algo with a long tail doesn't squash the
// rest. Always includes 0% so above/below-direct stays readable.
function rangeForRatios(ratios: number[]): SizeRange | null {
  if (ratios.length === 0) return null;
  let min = Infinity;
  let max = -Infinity;
  for (const v of ratios) {
    if (v < min) min = v;
    if (v > max) max = v;
  }
  if (!Number.isFinite(min) || !Number.isFinite(max)) return null;
  if (min > 0) min = 0;
  if (max < 0) max = 0;
  if (min === max) {
    min -= 1;
    max += 1;
  }
  const pad = (max - min) * 0.05;
  return { min: min - pad, max: max + pad };
}

function bin(ratios: number[], range: SizeRange, nBins: number): number[] {
  const bins = new Array(nBins).fill(0);
  const span = range.max - range.min;
  if (span <= 0) return bins;
  for (const r of ratios) {
    let i = Math.floor(((r - range.min) / span) * nBins);
    if (i < 0) i = 0;
    if (i >= nBins) i = nBins - 1;
    bins[i]++;
  }
  return bins;
}

function quantile(sorted: number[], q: number): number {
  if (sorted.length === 0) return 0;
  const i = (sorted.length - 1) * q;
  const lo = Math.floor(i);
  const hi = Math.ceil(i);
  if (lo === hi) return sorted[lo];
  return sorted[lo] * (hi - i) + sorted[hi] * (i - lo);
}

// Best algo per size = highest p25 of improvement_ratios. p25 captures
// "median quality" + "tail safety" in one number — algos with long
// negative tails auto-disqualify, and positive-only distributions
// differentiate by where their lower-quartile sits.
function bestAlgoFor(rows: BenchRow[]): string | null {
  let best: { algo: string; score: number } | null = null;
  for (const row of rows) {
    if (!row.improvement_ratios || row.improvement_ratios.length === 0) continue;
    const sorted = [...row.improvement_ratios].sort((a, b) => a - b);
    const score = quantile(sorted, 0.25);
    if (best === null || score > best.score) {
      best = { algo: row.algorithm, score };
    }
  }
  return best ? best.algo : null;
}

// Per-row latency histogram. Distribution is positive-only and usually
// right-skewed, so no zero reference line and a single neutral fill.
interface LatencyHistogramProps {
  times_ms: number[];
  color: string;
}

function LatencyHistogram({ times_ms, color }: LatencyHistogramProps) {
  if (times_ms.length === 0) {
    return <span className="hint">—</span>;
  }
  let min = Infinity;
  let max = -Infinity;
  for (const v of times_ms) {
    if (v < min) min = v;
    if (v > max) max = v;
  }
  if (!Number.isFinite(min) || !Number.isFinite(max)) {
    return <span className="hint">—</span>;
  }
  if (min === max) {
    max = min * 1.001 + 1e-9;
  }
  const bins = bin(times_ms, { min, max }, HIST_BINS);
  const peak = Math.max(1, ...bins);
  const binW = HIST_W / HIST_BINS;
  const sorted = [...times_ms].sort((a, b) => a - b);
  const fmt = (v: number) => {
    const f = formatDuration(v);
    return `${f.value} ${f.unit}`;
  };
  const tip =
    `n=${times_ms.length}\n` +
    `min ${fmt(sorted[0])}   p10 ${fmt(quantile(sorted, 0.1))}\n` +
    `med ${fmt(quantile(sorted, 0.5))}\n` +
    `p90 ${fmt(quantile(sorted, 0.9))}   max ${fmt(sorted[sorted.length - 1])}`;
  return (
    <div className="histogram-cell">
      <svg
        className="histogram"
        width={HIST_W}
        height={HIST_H}
        viewBox={`0 0 ${HIST_W} ${HIST_H}`}
      >
        <title>{tip}</title>
        {bins.map((count, i) => {
          if (count === 0) return null;
          const h = (count / peak) * (HIST_H - 2);
          return (
            <rect
              key={i}
              x={i * binW}
              y={HIST_H - h}
              width={Math.max(1, binW - 1)}
              height={h}
              fill={color}
              opacity={0.6}
            />
          );
        })}
      </svg>
      <div className="histogram-axis">
        <span>{fmt(sorted[0])}</span>
        <span>{fmt(sorted[sorted.length - 1])}</span>
      </div>
    </div>
  );
}

interface HistogramProps {
  ratios: number[];
  color: string;
}

function Histogram({ ratios, color }: HistogramProps) {
  if (ratios.length === 0) {
    return <span className="hint">—</span>;
  }
  const range = rangeForRatios(ratios);
  if (!range) return <span className="hint">—</span>;
  const bins = bin(ratios, range, HIST_BINS);
  const peak = Math.max(1, ...bins);
  const binW = HIST_W / HIST_BINS;
  const span = range.max - range.min;
  const zeroX = span > 0 ? ((0 - range.min) / span) * HIST_W : 0;
  const sorted = [...ratios].sort((a, b) => a - b);
  const p10 = quantile(sorted, 0.1);
  const p50 = quantile(sorted, 0.5);
  const p90 = quantile(sorted, 0.9);
  const tip =
    `n=${ratios.length}\n` +
    `min ${sorted[0].toFixed(2)}%   p10 ${p10.toFixed(2)}%\n` +
    `med ${p50.toFixed(2)}%\n` +
    `p90 ${p90.toFixed(2)}%   max ${sorted[sorted.length - 1].toFixed(2)}%`;
  return (
    <div className="histogram-cell">
      <svg
        className="histogram"
        width={HIST_W}
        height={HIST_H}
        viewBox={`0 0 ${HIST_W} ${HIST_H}`}
      >
        <title>{tip}</title>
        {/* zero reference line */}
        {zeroX >= 0 && zeroX <= HIST_W && (
          <line
            x1={zeroX}
            y1={0}
            x2={zeroX}
            y2={HIST_H}
            stroke="rgba(180, 180, 180, 0.35)"
            strokeDasharray="2 3"
            strokeWidth={1}
          />
        )}
        {bins.map((count, i) => {
          if (count === 0) return null;
          const h = (count / peak) * (HIST_H - 2);
          const binMid = range.min + ((i + 0.5) / HIST_BINS) * span;
          const fill = binMid >= 0 ? color : "#a04848";
          return (
            <rect
              key={i}
              x={i * binW}
              y={HIST_H - h}
              width={Math.max(1, binW - 1)}
              height={h}
              fill={fill}
              opacity={0.85}
            />
          );
        })}
      </svg>
      <div className="histogram-axis">
        <span>{sorted[0].toFixed(1)}%</span>
        <span>{sorted[sorted.length - 1].toFixed(1)}%</span>
      </div>
    </div>
  );
}

export function BenchmarksView() {
  const [state, setState] = useState<LoadState>({ kind: "loading" });

  useEffect(() => {
    let cancelled = false;
    const url = `${import.meta.env.BASE_URL}benchmarks.json`;
    fetch(url)
      .then(async (r) => {
        if (r.status === 404) {
          if (!cancelled) setState({ kind: "missing" });
          return;
        }
        if (!r.ok) throw new Error(`HTTP ${r.status}`);
        const report = (await r.json()) as BenchReport;
        if (!cancelled) setState({ kind: "ready", report });
      })
      .catch((e) => {
        if (!cancelled) setState({ kind: "error", message: String(e) });
      });
    return () => {
      cancelled = true;
    };
  }, []);

  if (state.kind === "loading") {
    return (
      <div className="bench-page">
        <p className="hint">Loading benchmarks…</p>
      </div>
    );
  }

  if (state.kind === "missing" || state.kind === "error") {
    return (
      <div className="bench-page">
        <section className="bench-empty">
          <h2>No benchmarks available</h2>
          <p className="hint">
            Run this to generate <code>web/public/benchmarks.json</code>:
          </p>
          <pre className="bench-cmd">
            cargo run --release -p routeviz-cli --bin bench --{"\n"}
            {"  "}--out web/public/benchmarks.json
          </pre>
          {state.kind === "error" && (
            <p className="warn">Error: {state.message}</p>
          )}
        </section>
      </div>
    );
  }

  const { report } = state;
  // Per-size cards with a global bar scale to show cross-size growth.
  const bySize = new Map<number, BenchRow[]>();
  for (const row of report.results) {
    const list = bySize.get(row.num_tokens) ?? [];
    list.push(row);
    bySize.set(row.num_tokens, list);
  }
  const globalMax = Math.max(
    1e-9,
    ...report.results.map((r) => r.median_ms),
  );

  const generated = new Date(report.generated_at_unix * 1000).toLocaleString();

  return (
    <div className="bench-page">
      <header className="bench-header">
        <div>
          <h2>Benchmarks</h2>
          <p className="hint">
            base seed {report.seed} · pair density {report.pair_density.toFixed(2)} ·
            {" "}
            {report.samples_per_config} samples per cell · generated {generated}
          </p>
          <p className="bench-note">
            Each sample generates a fresh graph (seed = base + i), picks
            one (src, dst, amount) pair at 10 % of the source pool&apos;s
            reserves, and times + scores every algo on it. Timings are
            <b> native Rust</b>; the browser runs a little slower due to
            WASM overhead. <b>vs direct</b> averages the net algo output
            against the best direct pool — positive means routing around
            slippage. The histogram shows the per-sample distribution.
          </p>
        </div>
      </header>

      {Array.from(bySize.entries())
        .sort(([a], [b]) => a - b)
        .map(([size, rows]) => {
          const sorted = [...rows].sort((a, b) => a.median_ms - b.median_ms);
          const numPools = rows[0]?.num_pools ?? 0;
          const winner = bestAlgoFor(rows);
          return (
            <section className="bench-group" key={size}>
              <h3>
                n = {size} <span className="hint">tokens · {numPools} pools</span>
              </h3>
              <table className="bench-table">
                <thead>
                  <tr>
                    <th className="bench-col-algo">Algorithm</th>
                    <th className="bench-col-bar" colSpan={2}>
                      Compute time (median)
                    </th>
                    <th className="bench-col-range">Time distribution</th>
                    <th className="bench-col-improvement">vs Direct</th>
                    <th className="bench-col-histogram">
                      Improvement distribution
                    </th>
                  </tr>
                </thead>
                <tbody>
                  {sorted.map((row) => {
                    const { name, color } = algoInfo(row.algorithm);
                    return (
                    <tr key={row.algorithm}>
                      <td className="algo-name">
                        <span
                          className="algo-swatch"
                          style={{ background: color }}
                        />
                        {name}
                        {winner === row.algorithm && (
                          <span
                            className="algo-star"
                            title="Best at this size — highest p25 of improvement vs direct (combines median quality and tail safety)"
                          >
                            ★
                          </span>
                        )}
                      </td>
                      <td className="bar-cell">
                        <div
                          className="bar"
                          style={{
                            width: `${(row.median_ms / globalMax) * 100}%`,
                            background: color,
                            color,
                          }}
                        />
                      </td>
                      <td className="bench-value">
                        {(() => {
                          const f = formatDuration(row.median_ms);
                          return `${f.value} ${f.unit}`;
                        })()}
                      </td>
                      <td className="bench-range">
                        {row.times_ms && row.times_ms.length > 0 ? (
                          <LatencyHistogram
                            times_ms={row.times_ms}
                            color={color}
                          />
                        ) : (
                          <span className="hint">—</span>
                        )}
                      </td>
                      <td className="bench-improvement">
                        {row.improvement_pct == null ? (
                          <span className="hint">—</span>
                        ) : (
                          <span
                            className={
                              row.improvement_pct > 0
                                ? "delta-pos mono"
                                : row.improvement_pct < 0
                                  ? "delta-neg mono"
                                  : "mono"
                            }
                          >
                            {row.improvement_pct > 0 ? "+" : ""}
                            {row.improvement_pct.toFixed(2)}%
                          </span>
                        )}
                      </td>
                      <td className="bench-histogram">
                        {row.improvement_ratios &&
                        row.improvement_ratios.length > 0 ? (
                          <Histogram
                            ratios={row.improvement_ratios}
                            color={color}
                          />
                        ) : (
                          <span className="hint">—</span>
                        )}
                      </td>
                    </tr>
                    );
                  })}
                </tbody>
              </table>
            </section>
          );
        })}
    </div>
  );
}
