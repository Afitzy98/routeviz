// Mirrors routeviz-core's serde shapes. U256 fields are 0x-strings —
// parse with BigInt() when math is needed.

export type Address = string;
export type U256Hex = string;
export type TokenKind = "Hub" | "Spoke";

export interface Token {
  address: Address;
  symbol: string;
  decimals: number;
  true_price_usd: number;
  kind: TokenKind;
}

export interface Pool {
  address: Address;
  token_a: Address;
  token_b: Address;
  reserve_a: U256Hex;
  reserve_b: U256Hex;
  fee_bps: number;
  venue: string;
}

export interface Point {
  x: number;
  y: number;
}

export interface GenConfig {
  version: number;
  num_tokens: number;
  price_noise: number;
  liquidity_spread: number;
  pair_density: number;
  seed: bigint;
}

export interface GeneratedGraph {
  tokens: Token[];
  pools: Pool[];
  positions: Point[];
  config: GenConfig;
}

// "1234.5678"-style render of a U256 hex amount.
export function formatUnits(hex: U256Hex, decimals: number, maxFractionalDigits = 4): string {
  const value = BigInt(hex);
  if (value === 0n) return "0";
  const base = 10n ** BigInt(decimals);
  const whole = value / base;
  const frac = value % base;
  if (frac === 0n) return whole.toString();
  const fracStr = frac.toString().padStart(decimals, "0");
  const trimmed = fracStr.slice(0, maxFractionalDigits).replace(/0+$/, "");
  return trimmed.length > 0 ? `${whole}.${trimmed}` : whole.toString();
}

export function shortAddress(addr: Address, head = 6, tail = 4): string {
  if (addr.length <= head + tail + 2) return addr;
  return `${addr.slice(0, head)}…${addr.slice(-tail)}`;
}

// --- algorithm types ---

export type AlgorithmId =
  | "dijkstra"
  | "bellman_ford"
  | "amount_aware"
  | "split_dp"
  | "split_fw";

export type LayoutMode = "hub_spoke" | "force_directed";

export interface AlgorithmInfo {
  id: AlgorithmId;
  name: string;
  available: boolean;
  color: string;
  blurb: string;
}

export const ALGORITHMS: AlgorithmInfo[] = [
  {
    id: "dijkstra",
    name: "Dijkstra",
    available: true,
    color: "#c9a227",
    blurb: "Shortest log-weighted route. Marginal rates only; ignores slippage.",
  },
  {
    id: "bellman_ford",
    name: "Bellman-Ford",
    available: true,
    color: "#d4523f",
    blurb: "V-1 relaxation passes. Detects negative cycles (arbitrage).",
  },
  {
    id: "amount_aware",
    name: "Amount-aware",
    available: true,
    color: "#5cb775",
    blurb: "Top-K bounded-hop Bellman-Ford shortlist (≤3 hops) rescored by exact U256 simulate_path. The production pattern (cf. 1inch Pathfinder): linear in edges per hop, correct on negative edges and cycles.",
  },
  {
    id: "split_dp",
    name: "Split (DP)",
    available: true,
    color: "#e08a5c",
    blurb: "Top-K candidate paths × chunked input × knapsack DP. What Uniswap Smart Order Router ships. Approximates shared-pool interference; trades made of disjoint paths are exact.",
  },
  {
    id: "split_fw",
    name: "Split (Frank-Wolfe)",
    available: true,
    color: "#c87de0",
    blurb: "Convex-optimisation split router. Each iteration recomputes pool reserves from the committed flow, then runs shortest-path on the updated marginal-price graph to pick the next direction. Correct under shared-pool interference; the CFMM-routing paper version (Angeris et al.).",
  },
];

export type Step =
  | { Visit: number }
  | { Relax: { from: number; to: number; new_distance: number } }
  | { Pass: number };

export interface Leg {
  path: Address[];
  pools_used: Address[];
  amount_in: U256Hex;
  amount_out: U256Hex;
}

export type Outcome =
  | {
      Found: {
        path: Address[];
        pools_used: Address[];
        total_log_weight: number;
        product_of_rates: number;
        amount_in: U256Hex;
        amount_out: U256Hex;
        /** Gas cost in dst-token base units. Computed by core's
         * GasModel using the request's `gas_price_gwei`. Zero when
         * gas accounting is disabled. UI computes net = amount_out
         * − gas_cost. */
        gas_cost: U256Hex;
      };
    }
  | {
      FoundSplit: {
        legs: Leg[];
        amount_in: U256Hex;
        amount_out: U256Hex;
        gas_cost: U256Hex;
      };
    }
  | {
      NegativeCycle: {
        cycle: Address[];
        pools_used: Address[];
        product_of_rates: number;
        amount_in: U256Hex;
        cycle_output: U256Hex;
        gas_cost: U256Hex;
      };
    }
  | "NoPath";

export interface SolveResult {
  outcome: Outcome;
  trace: Step[];
}

// "1.5" + 18 → 0x... (1.5 × 10^18). "" / invalid → "0x0".
export function parseUnits(amount: string, decimals: number): U256Hex {
  if (!amount) return "0x0";
  const trimmed = amount.trim();
  if (!/^\d+(\.\d*)?$/.test(trimmed)) return "0x0";
  const [whole, frac = ""] = trimmed.split(".");
  const fracPadded = (frac + "0".repeat(decimals)).slice(0, decimals);
  const combined = (whole + fracPadded).replace(/^0+/, "") || "0";
  const value = BigInt(combined);
  return "0x" + value.toString(16);
}

// USD → token base units. Bypasses parseUnits because float division
// can produce scientific notation parseUnits can't read.
export function usdToBaseUnits(
  usd: number,
  priceUsd: number,
  decimals: number,
): bigint | null {
  if (!Number.isFinite(usd) || usd <= 0) return null;
  if (!Number.isFinite(priceUsd) || priceUsd <= 0) return null;
  const human = usd / priceUsd;
  const scale = 10n ** BigInt(decimals);
  const whole = BigInt(Math.floor(human));
  const fracFloat = human - Math.floor(human);
  const fracScaled = BigInt(Math.floor(fracFloat * Number(scale)));
  const wei = whole * scale + fracScaled;
  return wei === 0n ? null : wei;
}

