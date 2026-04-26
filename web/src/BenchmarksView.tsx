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
}

interface BenchReport {
  generated_at_unix: number;
  seed: number;
  pair_density: number;
  sizes: number[];
  samples_per_config: number;
  improvement_pairs?: number;
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
            seed {report.seed} · pair density {report.pair_density.toFixed(2)} ·
            {" "}
            {report.samples_per_config} samples per config · generated{" "}
            {generated}
          </p>
          <p className="bench-note">
            Timings from the <b>native Rust</b> binary; the browser runs
            a little slower due to WASM overhead. The <b>vs direct</b>
            {" "}column averages algo output against the best direct-pool
            swap across{" "}
            {report.improvement_pairs ?? 40} random (src, dst) pairs per
            cell at 10 % of the pool&apos;s reserves — so positive means
            the algo routes around slippage better than taking the direct
            pool.
          </p>
        </div>
      </header>

      {Array.from(bySize.entries())
        .sort(([a], [b]) => a - b)
        .map(([size, rows]) => {
          const sorted = [...rows].sort((a, b) => a.median_ms - b.median_ms);
          const numPools = rows[0]?.num_pools ?? 0;
          return (
            <section className="bench-group" key={size}>
              <h3>
                n = {size} <span className="hint">tokens · {numPools} pools</span>
              </h3>
              <table className="bench-table">
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
                        {(() => {
                          const unit = formatDuration(row.max_ms).unit;
                          const scale =
                            unit === "µs" ? 1000 : unit === "s" ? 1 / 1000 : 1;
                          return `${(row.min_ms * scale).toFixed(2)} – ${(
                            row.max_ms * scale
                          ).toFixed(2)} ${unit}`;
                        })()}
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
