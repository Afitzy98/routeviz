import type { GenConfig, LayoutMode } from "./types";

interface Props {
  config: GenConfig;
  lockSeed: boolean;
  setLockSeed: (v: boolean) => void;
  updateConfig: (patch: Partial<GenConfig>) => void;
  regenerate: () => void;
  layoutMode: LayoutMode;
  changeLayout: (m: LayoutMode) => void;
  showVenuesHint?: boolean;
  onBeforeRegenerate?: () => void;
  gasPriceGwei: number;
  setGasPriceGwei: (v: number) => void;
}

// Must match core/src/algo/gas.rs exactly.
const BASE_TX_GAS = 21_000;
const ROUTER_OVERHEAD_GAS = 30_000;
const PER_HOP_GAS = 90_000;
const ETH_PRICE_USD = 3000;
const SINGLE_SWAP_GAS = BASE_TX_GAS + ROUTER_OVERHEAD_GAS + PER_HOP_GAS;

export function GraphPanel({
  config,
  lockSeed,
  setLockSeed,
  updateConfig,
  regenerate,
  layoutMode,
  changeLayout,
  showVenuesHint = false,
  onBeforeRegenerate,
  gasPriceGwei,
  setGasPriceGwei,
}: Props) {
  const singleSwapUsd =
    SINGLE_SWAP_GAS * gasPriceGwei * 1e-9 * ETH_PRICE_USD;
  return (
    <section className="panel">
      <h3>Graph</h3>
      <label>
        <span>Tokens</span>
        <input
          type="number"
          min={5}
          max={1000}
          step={5}
          value={config.num_tokens}
          onChange={(e) =>
            updateConfig({
              num_tokens: Math.max(5, Math.min(1000, +e.target.value || 5)),
            })
          }
        />
      </label>
      <label>
        <span>Pair density</span>
        <span className="mono small">{config.pair_density.toFixed(2)}</span>
      </label>
      <input
        type="range"
        min={0.05}
        max={1}
        step={0.01}
        value={config.pair_density}
        onChange={(e) => updateConfig({ pair_density: +e.target.value })}
      />
      <label>
        <span>Price noise</span>
        <span className="mono small">{config.price_noise.toFixed(3)}</span>
      </label>
      <input
        type="range"
        min={0}
        max={0.1}
        step={0.001}
        value={config.price_noise}
        onChange={(e) => updateConfig({ price_noise: +e.target.value })}
      />
      {showVenuesHint && (
        <p className="hint">
          Pools live on multiple V2-style venues (Uniswap V2, SushiSwap,
          PancakeSwap, Biswap) — each venue has its own fee. Hover any
          edge to see which venue it belongs to.
        </p>
      )}
      <label>
        <span>Gas price</span>
        <span className="mono small">
          {gasPriceGwei.toFixed(0)} gwei
        </span>
      </label>
      <input
        type="range"
        min={0}
        max={200}
        step={1}
        value={gasPriceGwei}
        onChange={(e) => setGasPriceGwei(+e.target.value)}
      />
      <p className="hint">
        {gasPriceGwei === 0 ? (
          <>Gas accounting off — algorithms optimise on gross output.</>
        ) : (
          <>
            ≈{" "}
            <span className="mono">
              ${singleSwapUsd.toFixed(singleSwapUsd < 1 ? 3 : 2)}
            </span>{" "}
            per single-hop swap. Each split leg pays this; algorithms
            now optimise on output net of gas.
          </>
        )}
      </p>
      <label className="checkbox-row">
        <input
          type="checkbox"
          checked={lockSeed}
          onChange={(e) => setLockSeed(e.target.checked)}
        />
        <span>Lock seed</span>
      </label>
      <div className="pick-mode">
        <span className="hint">Layout</span>
        <div className="toggle">
          <button
            className={layoutMode === "hub_spoke" ? "active" : ""}
            onClick={() => changeLayout("hub_spoke")}
          >
            Hub-spoke
          </button>
          <button
            className={layoutMode === "force_directed" ? "active" : ""}
            onClick={() => changeLayout("force_directed")}
          >
            Force
          </button>
        </div>
      </div>
      <button
        className="btn btn-primary btn-full"
        onClick={() => {
          onBeforeRegenerate?.();
          regenerate();
        }}
      >
        ⚡ Generate
      </button>
    </section>
  );
}
