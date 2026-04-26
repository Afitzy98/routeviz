import { useEffect, useMemo, useRef, useState } from "react";
import type {
  Address,
  GeneratedGraph,
  Pool,
  SolveResult,
  Step,
  Token,
} from "./types";
import { formatUnits, shortAddress } from "./types";

interface Props {
  graph: GeneratedGraph | null;
  result?: SolveResult | null;
  animIndex?: number;
  showPath?: boolean;
  src?: Address | null;
  dst?: Address | null;
  onTokenClick?: (addr: Address, shift: boolean) => void;
  logicalWidth: number;
  logicalHeight: number;
  /** How to colour a detected cycle. "loss" (red) is the default and
   * fits the Router tab — there a cycle interrupts the user's routing
   * query, so we treat it as a warning. "profit" (green) fits the
   * Arbitrage tab — the cycle is exactly what the user asked for and
   * the colour signals "this is the prize". */
  cycleStyle?: "loss" | "profit";
}

const COLORS = {
  edge: "rgba(201, 162, 39, 0.32)",
  edgeNeighbor: "rgba(240, 198, 71, 0.6)",
  edgeRelaxed: "rgba(240, 198, 71, 0.5)",
  edgeActive: "#f0c647",
  edgePath: "#c9a227",
  edgeCycle: "#d4523f",
  edgeCycleProfit: "#5cb775",
  edgeHover: "rgba(240, 198, 71, 0.9)",
  spokeFill: "#1a2030",
  spokeStroke: "#3d4454",
  spokeText: "#9a9484",
  hubFill: "#2a2418",
  hubStroke: "#c9a227",
  hubText: "#f0c647",
  visitedFill: "#2f2a16",
  visitedStroke: "#a87f2e",
  visitedText: "#e9d289",
  srcFill: "#1a2a21",
  srcStroke: "#5cb775",
  srcText: "#b6e4c1",
  dstFill: "#2a1814",
  dstStroke: "#d4523f",
  dstText: "#ffcab9",
  pathFill: "#c9a227",
  pathStroke: "#f0c647",
  pathText: "#1a1308",
  cycleFill: "#d4523f",
  cycleStroke: "#ff8872",
  cycleText: "#ffe7df",
  cycleProfitFill: "#1f3a25",
  cycleProfitStroke: "#5cb775",
  cycleProfitText: "#cdf2d6",
  hoverFill: "#c9a227",
  hoverStroke: "#f0c647",
  hoverText: "#1a1308",
};

const HUB_RADIUS_MULTIPLIER = 1.55;

interface HoverNode {
  kind: "node";
  index: number;
}
interface HoverEdge {
  kind: "edge";
  poolIdx: number;
}
type Hover = HoverNode | HoverEdge | null;

type NodeStatus = "unvisited" | "visited" | "src" | "dst" | "path" | "cycle";

const edgeKey = (a: number, b: number) =>
  a < b ? `${a}-${b}` : `${b}-${a}`;

export function TokenGraphCanvas({
  graph,
  result = null,
  animIndex = 0,
  showPath = false,
  src = null,
  dst = null,
  onTokenClick,
  logicalWidth,
  logicalHeight,
  cycleStyle = "loss",
}: Props) {
  // "loss" = red (Router), "profit" = green (Arb).
  const cycleEdgeColor =
    cycleStyle === "profit" ? COLORS.edgeCycleProfit : COLORS.edgeCycle;
  const cycleFill =
    cycleStyle === "profit" ? COLORS.cycleProfitFill : COLORS.cycleFill;
  const cycleStroke =
    cycleStyle === "profit" ? COLORS.cycleProfitStroke : COLORS.cycleStroke;
  const cycleText =
    cycleStyle === "profit" ? COLORS.cycleProfitText : COLORS.cycleText;
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const wrapRef = useRef<HTMLDivElement>(null);
  const [size, setSize] = useState({ w: 800, h: 600 });
  const [hover, setHover] = useState<Hover>(null);
  const [mouse, setMouse] = useState<{ x: number; y: number } | null>(null);

  useEffect(() => {
    const el = wrapRef.current;
    if (!el) return;
    const ro = new ResizeObserver((entries) => {
      const r = entries[0].contentRect;
      setSize({ w: Math.max(200, r.width), h: Math.max(200, r.height) });
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const { scaleX, scaleY, offsetX, offsetY } = useMemo(() => {
    const pad = 50;
    const availW = size.w - pad * 2;
    const availH = size.h - pad * 2;
    const s = Math.min(availW / logicalWidth, availH / logicalHeight);
    return {
      scaleX: s,
      scaleY: s,
      offsetX: (size.w - logicalWidth * s) / 2,
      offsetY: (size.h - logicalHeight * s) / 2,
    };
  }, [size, logicalWidth, logicalHeight]);

  const nodeRadius = useMemo(() => {
    const n = graph?.tokens.length ?? 0;
    if (n === 0) return 14;
    return Math.max(6, Math.min(28, 220 / Math.sqrt(n)));
  }, [graph]);

  const tokenIndex = useMemo(() => {
    const m = new Map<string, number>();
    if (graph) graph.tokens.forEach((t, i) => m.set(t.address, i));
    return m;
  }, [graph]);

  // Spread parallel pools as separated arcs rather than overlapping.
  const curveOffsets = useMemo(() => {
    const offsets: number[] = new Array(graph?.pools.length ?? 0).fill(0);
    if (!graph) return offsets;
    const groups = new Map<string, number[]>();
    for (let i = 0; i < graph.pools.length; i++) {
      const p = graph.pools[i];
      const key = edgeKey(
        tokenIndex.get(p.token_a) ?? -1,
        tokenIndex.get(p.token_b) ?? -1,
      );
      const g = groups.get(key) ?? [];
      g.push(i);
      groups.set(key, g);
    }
    for (const members of groups.values()) {
      const n = members.length;
      if (n <= 1) continue;
      for (let i = 0; i < n; i++) {
        offsets[members[i]] = 2 * i - (n - 1);
      }
    }
    return offsets;
  }, [graph, tokenIndex]);

  // Per-frame visualisation state.
  const derived = useMemo(() => {
    const visited = new Set<number>();
    const relaxedEdges = new Set<string>();
    let activeNode: number | null = null;
    let activeEdge: string | null = null;
    const pathSet = new Set<number>();
    const pathEdgeSet = new Set<string>();
    const pathPools = new Set<Address>();
    // FoundSplit: per-pool flow fraction → stroke-width scale.
    const poolFlow = new Map<Address, number>();
    const cycleSet = new Set<number>();
    const cycleEdgeSet = new Set<string>();
    const cyclePools = new Set<Address>();

    if (!graph || !result) {
      return {
        visited,
        relaxedEdges,
        activeNode,
        activeEdge,
        pathSet,
        pathEdgeSet,
        pathPools,
        poolFlow,
        cycleSet,
        cycleEdgeSet,
        cyclePools,
      };
    }

    const upto = Math.min(animIndex, result.trace.length);
    for (let i = 0; i < upto; i++) {
      const step = result.trace[i];
      if ("Visit" in step) {
        visited.add(step.Visit);
      } else if ("Relax" in step) {
        relaxedEdges.add(edgeKey(step.Relax.from, step.Relax.to));
      }
    }
    if (animIndex > 0 && animIndex <= result.trace.length) {
      const active = result.trace[animIndex - 1];
      if ("Visit" in active) {
        activeNode = active.Visit;
      } else if ("Relax" in active) {
        activeEdge = edgeKey(active.Relax.from, active.Relax.to);
        activeNode = active.Relax.to;
      }
    }

    if (showPath && typeof result.outcome === "object") {
      if ("Found" in result.outcome) {
        const found = result.outcome.Found;
        for (const a of found.path) {
          const i = tokenIndex.get(a);
          if (i != null) pathSet.add(i);
        }
        for (let i = 0; i < found.path.length - 1; i++) {
          const a = tokenIndex.get(found.path[i]);
          const b = tokenIndex.get(found.path[i + 1]);
          if (a != null && b != null) pathEdgeSet.add(edgeKey(a, b));
        }
        for (const pool of found.pools_used) pathPools.add(pool);
      } else if ("FoundSplit" in result.outcome) {
        const split = result.outcome.FoundSplit;
        const totalIn = BigInt(split.amount_in);
        for (const leg of split.legs) {
          for (const a of leg.path) {
            const i = tokenIndex.get(a);
            if (i != null) pathSet.add(i);
          }
          for (let i = 0; i < leg.path.length - 1; i++) {
            const a = tokenIndex.get(leg.path[i]);
            const b = tokenIndex.get(leg.path[i + 1]);
            if (a != null && b != null) pathEdgeSet.add(edgeKey(a, b));
          }
          // BigInt math — Number() can overflow at 18 decimals.
          const frac =
            totalIn === 0n
              ? 0
              : Number((BigInt(leg.amount_in) * 1000n) / totalIn) / 1000;
          for (const pool of leg.pools_used) {
            pathPools.add(pool);
            poolFlow.set(pool, (poolFlow.get(pool) ?? 0) + frac);
          }
        }
      } else if ("NegativeCycle" in result.outcome) {
        const cy = result.outcome.NegativeCycle;
        for (const a of cy.cycle) {
          const i = tokenIndex.get(a);
          if (i != null) cycleSet.add(i);
        }
        for (let i = 0; i < cy.cycle.length; i++) {
          const a = tokenIndex.get(cy.cycle[i]);
          const b = tokenIndex.get(cy.cycle[(i + 1) % cy.cycle.length]);
          if (a != null && b != null) cycleEdgeSet.add(edgeKey(a, b));
        }
        for (const pool of cy.pools_used) cyclePools.add(pool);
      }
    }

    return {
      visited,
      relaxedEdges,
      activeNode,
      activeEdge,
      pathSet,
      pathEdgeSet,
      pathPools,
      poolFlow,
      cycleSet,
      cycleEdgeSet,
      cyclePools,
    };
  }, [graph, result, animIndex, showPath, tokenIndex]);

  const project = (x: number, y: number) => ({
    x: (x + logicalWidth / 2) * scaleX + offsetX,
    y: (y + logicalHeight / 2) * scaleY + offsetY,
  });

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    const dpr = window.devicePixelRatio || 1;
    canvas.width = Math.round(size.w * dpr);
    canvas.height = Math.round(size.h * dpr);
    canvas.style.width = `${size.w}px`;
    canvas.style.height = `${size.h}px`;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, size.w, size.h);

    if (!graph || graph.tokens.length === 0) {
      ctx.fillStyle = "rgba(155, 135, 95, 0.45)";
      ctx.font = "14px ui-sans-serif, system-ui, sans-serif";
      ctx.textAlign = "center";
      ctx.textBaseline = "middle";
      ctx.fillText("Generate a graph to begin", size.w / 2, size.h / 2);
      return;
    }

    ctx.lineCap = "round";

    const hoveredPoolIdx = hover?.kind === "edge" ? hover.poolIdx : -1;
    const hoveredNodeIdx = hover?.kind === "node" ? hover.index : -1;

    // Painter-style: later passes overdraw earlier ones, so the
    // priority order is base < neighbor < relaxed < path < cycle < active.
    type EdgeStyle =
      | "base"
      | "neighbor"
      | "relaxed"
      | "path"
      | "cycle"
      | "active";
    const byStyle: Record<EdgeStyle, number[]> = {
      base: [],
      neighbor: [],
      relaxed: [],
      path: [],
      cycle: [],
      active: [],
    };
    for (let pIdx = 0; pIdx < graph.pools.length; pIdx++) {
      const pool = graph.pools[pIdx];
      const aIdx = tokenIndex.get(pool.token_a);
      const bIdx = tokenIndex.get(pool.token_b);
      if (aIdx == null || bIdx == null) continue;
      const ek = edgeKey(aIdx, bIdx);
      const touchesHover =
        hoveredNodeIdx >= 0 && (aIdx === hoveredNodeIdx || bIdx === hoveredNodeIdx);

      let style: EdgeStyle = "base";
      if (derived.cyclePools.has(pool.address)) {
        style = "cycle";
      } else if (derived.pathPools.has(pool.address)) {
        style = "path";
      } else if (derived.activeEdge === ek) {
        style = "active";
      } else if (derived.relaxedEdges.has(ek)) {
        style = "relaxed";
      } else if (touchesHover) {
        style = "neighbor";
      }
      byStyle[style].push(pIdx);
    }

    // Pixels per parallel-arc step.
    const ARC_STEP_PX = 14;

    const drawEdges = (
      idxs: number[],
      color: string,
      width: number,
      glow: number,
    ) => {
      if (idxs.length === 0) return;
      ctx.save();
      if (glow > 0) {
        ctx.shadowColor = color;
        ctx.shadowBlur = glow;
      }
      ctx.strokeStyle = color;
      ctx.lineWidth = width;
      for (const idx of idxs) {
        const pool = graph.pools[idx];
        const aIdx = tokenIndex.get(pool.token_a)!;
        const bIdx = tokenIndex.get(pool.token_b)!;
        const p1 = project(graph.positions[aIdx].x, graph.positions[aIdx].y);
        const p2 = project(graph.positions[bIdx].x, graph.positions[bIdx].y);
        ctx.beginPath();
        ctx.moveTo(p1.x, p1.y);
        const offset = curveOffsets[idx] ?? 0;
        if (offset === 0) {
          ctx.lineTo(p2.x, p2.y);
        } else {
          // Bezier with a perpendicular control point.
          const dx = p2.x - p1.x;
          const dy = p2.y - p1.y;
          const len = Math.sqrt(dx * dx + dy * dy) || 1;
          // Apex sits at half the control-point distance, so ×2.
          const perpX = -dy / len;
          const perpY = dx / len;
          const mx = (p1.x + p2.x) / 2 + perpX * offset * ARC_STEP_PX * 2;
          const my = (p1.y + p2.y) / 2 + perpY * offset * ARC_STEP_PX * 2;
          ctx.quadraticCurveTo(mx, my, p2.x, p2.y);
        }
        ctx.stroke();
      }
      ctx.restore();
    };

    drawEdges(byStyle.base, COLORS.edge, 1, 0);
    drawEdges(byStyle.neighbor, COLORS.edgeNeighbor, 1.5, 4);
    drawEdges(byStyle.relaxed, COLORS.edgeRelaxed, 1.5, 6);
    // FoundSplit: stroke width scales with the leg's flow fraction.
    if (derived.poolFlow.size > 0) {
      for (const idx of byStyle.path) {
        const pool = graph.pools[idx];
        const frac = derived.poolFlow.get(pool.address) ?? 0;
        // 100% flow → 6px; 10% → 1.5px floor.
        const w = Math.max(1.5, Math.min(6, 6 * frac));
        drawEdges([idx], COLORS.edgePath, w, 16);
      }
    } else {
      drawEdges(byStyle.path, COLORS.edgePath, 3, 16);
    }
    drawEdges(byStyle.cycle, cycleEdgeColor, 3, 18);
    drawEdges(byStyle.active, COLORS.edgeActive, 2.5, 14);

    // Hovered edge on top of everything else.
    if (hoveredPoolIdx >= 0) {
      drawEdges([hoveredPoolIdx], COLORS.edgeHover, 2.5, 10);
    }

    // Nodes.
    ctx.textAlign = "center";
    ctx.textBaseline = "middle";
    const srcIdx = src ? (tokenIndex.get(src) ?? -1) : -1;
    const dstIdx = dst ? (tokenIndex.get(dst) ?? -1) : -1;

    for (let i = 0; i < graph.tokens.length; i++) {
      const tok = graph.tokens[i];
      const pt = graph.positions[i];
      const p = project(pt.x, pt.y);
      const isHovered = i === hoveredNodeIdx;
      const isActive = i === derived.activeNode && !showPath;
      const isHub = tok.kind === "Hub";
      const baseRadius = isHub ? nodeRadius * HUB_RADIUS_MULTIPLIER : nodeRadius;

      const status: NodeStatus =
        i === srcIdx
          ? "src"
          : i === dstIdx
            ? "dst"
            : showPath && derived.cycleSet.has(i)
              ? "cycle"
              : showPath && derived.pathSet.has(i)
                ? "path"
                : derived.visited.has(i)
                  ? "visited"
                  : "unvisited";

      let fill: string;
      let stroke: string;
      let textColor: string;
      let glow = 0;

      if (isHovered) {
        fill = COLORS.hoverFill;
        stroke = COLORS.hoverStroke;
        textColor = COLORS.hoverText;
        glow = 20;
      } else if (status === "src") {
        fill = COLORS.srcFill;
        stroke = COLORS.srcStroke;
        textColor = COLORS.srcText;
        glow = 18;
      } else if (status === "dst") {
        fill = COLORS.dstFill;
        stroke = COLORS.dstStroke;
        textColor = COLORS.dstText;
        glow = 18;
      } else if (status === "cycle") {
        fill = cycleFill;
        stroke = cycleStroke;
        textColor = cycleText;
        glow = 18;
      } else if (status === "path") {
        fill = COLORS.pathFill;
        stroke = COLORS.pathStroke;
        textColor = COLORS.pathText;
        glow = 16;
      } else if (status === "visited") {
        fill = COLORS.visitedFill;
        stroke = COLORS.visitedStroke;
        textColor = COLORS.visitedText;
        glow = 8;
      } else if (isHub) {
        fill = COLORS.hubFill;
        stroke = COLORS.hubStroke;
        textColor = COLORS.hubText;
        glow = 10;
      } else {
        fill = COLORS.spokeFill;
        stroke = COLORS.spokeStroke;
        textColor = COLORS.spokeText;
      }

      ctx.save();
      if (glow > 0) {
        ctx.shadowColor = stroke;
        ctx.shadowBlur = glow;
      }
      ctx.fillStyle = fill;
      ctx.beginPath();
      ctx.arc(p.x, p.y, baseRadius, 0, Math.PI * 2);
      ctx.fill();
      ctx.restore();

      ctx.strokeStyle = stroke;
      ctx.lineWidth = isHub || status === "src" || status === "dst" ? 2 : 1.5;
      ctx.beginPath();
      ctx.arc(p.x, p.y, baseRadius, 0, Math.PI * 2);
      ctx.stroke();

      // Active-step ring.
      if (isActive) {
        ctx.save();
        ctx.strokeStyle = COLORS.edgeActive;
        ctx.shadowColor = COLORS.edgeActive;
        ctx.shadowBlur = 12;
        ctx.lineWidth = 2;
        ctx.beginPath();
        ctx.arc(p.x, p.y, baseRadius + 4, 0, Math.PI * 2);
        ctx.stroke();
        ctx.restore();
      }

      const labelFontSize = Math.max(
        9,
        Math.floor(baseRadius * (isHub ? 0.6 : 0.55)),
      );
      const showLabel =
        isHub ||
        isHovered ||
        status === "src" ||
        status === "dst" ||
        status === "path" ||
        status === "cycle" ||
        baseRadius >= 10;
      if (showLabel) {
        ctx.font = `${isHub ? 700 : 600} ${labelFontSize}px ui-monospace, Menlo, Consolas, monospace`;
        ctx.fillStyle = textColor;
        ctx.fillText(tok.symbol, p.x, p.y + 0.5);
      }
    }
  }, [
    graph,
    size,
    scaleX,
    scaleY,
    offsetX,
    offsetY,
    nodeRadius,
    tokenIndex,
    hover,
    derived,
    showPath,
    src,
    dst,
    logicalWidth,
    logicalHeight,
  ]);

  const pickEntity = (mx: number, my: number): Hover => {
    if (!graph) return null;
    for (let i = 0; i < graph.tokens.length; i++) {
      const p = project(graph.positions[i].x, graph.positions[i].y);
      const dx = p.x - mx;
      const dy = p.y - my;
      const effectiveR =
        (graph.tokens[i].kind === "Hub"
          ? nodeRadius * HUB_RADIUS_MULTIPLIER
          : nodeRadius) + 4;
      if (dx * dx + dy * dy <= effectiveR * effectiveR) {
        return { kind: "node", index: i };
      }
    }
    let bestPool = -1;
    let bestDist = 8;
    const ARC_STEP_PX = 14;
    for (let i = 0; i < graph.pools.length; i++) {
      const pool = graph.pools[i];
      const aIdx = tokenIndex.get(pool.token_a);
      const bIdx = tokenIndex.get(pool.token_b);
      if (aIdx == null || bIdx == null) continue;
      const p1 = project(graph.positions[aIdx].x, graph.positions[aIdx].y);
      const p2 = project(graph.positions[bIdx].x, graph.positions[bIdx].y);
      const offset = curveOffsets[i] ?? 0;
      const d =
        offset === 0
          ? distToSegment(mx, my, p1.x, p1.y, p2.x, p2.y)
          : distToQuadraticBezier(mx, my, p1, p2, offset, ARC_STEP_PX);
      if (d < bestDist) {
        bestDist = d;
        bestPool = i;
      }
    }
    if (bestPool >= 0) return { kind: "edge", poolIdx: bestPool };
    return null;
  };

  const onMouseMove = (e: React.MouseEvent<HTMLCanvasElement>) => {
    const rect = e.currentTarget.getBoundingClientRect();
    const mx = e.clientX - rect.left;
    const my = e.clientY - rect.top;
    setMouse({ x: mx, y: my });
    setHover(pickEntity(mx, my));
  };

  const onMouseLeave = () => {
    setHover(null);
    setMouse(null);
  };

  const onClick = (e: React.MouseEvent<HTMLCanvasElement>) => {
    if (!graph || !onTokenClick) return;
    const h = hover;
    if (h && h.kind === "node") {
      onTokenClick(graph.tokens[h.index].address, e.shiftKey);
    }
  };

  return (
    <div ref={wrapRef} className="canvas-wrap">
      <canvas
        ref={canvasRef}
        onMouseMove={onMouseMove}
        onMouseLeave={onMouseLeave}
        onClick={onClick}
      />
      {hover && mouse && graph && (
        <Tooltip
          x={mouse.x}
          y={mouse.y}
          containerSize={size}
          content={tooltipContent(hover, graph)}
        />
      )}
    </div>
  );
}

function tooltipContent(hover: HoverNode | HoverEdge, graph: GeneratedGraph) {
  if (hover.kind === "node") {
    const tok = graph.tokens[hover.index];
    return (
      <>
        <div className="tip-title">
          {tok.symbol}
          <span className={`kind-chip kind-${tok.kind.toLowerCase()}`}>
            {tok.kind.toLowerCase()}
          </span>
        </div>
        <div className="tip-row">
          <span>address</span>
          <span className="mono">{shortAddress(tok.address)}</span>
        </div>
        <div className="tip-row">
          <span>decimals</span>
          <span className="mono">{tok.decimals}</span>
        </div>
        <div className="tip-row">
          <span>price</span>
          <span className="mono">${tok.true_price_usd.toFixed(4)}</span>
        </div>
      </>
    );
  }
  const pool = graph.pools[hover.poolIdx];
  const tokenA = graph.tokens.find((t) => t.address === pool.token_a);
  const tokenB = graph.tokens.find((t) => t.address === pool.token_b);
  const decA = tokenA?.decimals ?? 18;
  const decB = tokenB?.decimals ?? 18;
  return (
    <>
      <div className="tip-title">
        {tokenA?.symbol} / {tokenB?.symbol}
      </div>
      <div className="tip-row">
        <span>venue</span>
        <span>{pool.venue}</span>
      </div>
      <div className="tip-row">
        <span>pool</span>
        <span className="mono">{shortAddress(pool.address)}</span>
      </div>
      <div className="tip-row">
        <span>fee</span>
        <span className="mono">{(pool.fee_bps / 100).toFixed(2)}%</span>
      </div>
      <div className="tip-row">
        <span>reserve A</span>
        <span className="mono">
          {formatUnits(pool.reserve_a, decA)} {tokenA?.symbol}
        </span>
      </div>
      <div className="tip-row">
        <span>reserve B</span>
        <span className="mono">
          {formatUnits(pool.reserve_b, decB)} {tokenB?.symbol}
        </span>
      </div>
    </>
  );
}

function Tooltip({
  x,
  y,
  containerSize,
  content,
}: {
  x: number;
  y: number;
  containerSize: { w: number; h: number };
  content: React.ReactNode;
}) {
  const padding = 14;
  const approxW = 260;
  const approxH = 140;
  const left =
    x + approxW + padding > containerSize.w ? x - approxW - padding : x + padding;
  const top =
    y + approxH + padding > containerSize.h
      ? y - approxH - padding
      : y + padding;
  return (
    <div className="tooltip" style={{ left, top }}>
      {content}
    </div>
  );
}

// Sampled distance from point to the parallel-pool Bezier — close
// enough for hover hit-testing.
function distToQuadraticBezier(
  px: number,
  py: number,
  p1: { x: number; y: number },
  p2: { x: number; y: number },
  offset: number,
  step: number,
): number {
  const dx = p2.x - p1.x;
  const dy = p2.y - p1.y;
  const len = Math.sqrt(dx * dx + dy * dy) || 1;
  const perpX = -dy / len;
  const perpY = dx / len;
  const cx = (p1.x + p2.x) / 2 + perpX * offset * step * 2;
  const cy = (p1.y + p2.y) / 2 + perpY * offset * step * 2;
  const samples = 12;
  let prevX = p1.x;
  let prevY = p1.y;
  let best = Infinity;
  for (let i = 1; i <= samples; i++) {
    const t = i / samples;
    const mt = 1 - t;
    const x = mt * mt * p1.x + 2 * mt * t * cx + t * t * p2.x;
    const y = mt * mt * p1.y + 2 * mt * t * cy + t * t * p2.y;
    const d = distToSegment(px, py, prevX, prevY, x, y);
    if (d < best) best = d;
    prevX = x;
    prevY = y;
  }
  return best;
}

function distToSegment(
  px: number,
  py: number,
  x1: number,
  y1: number,
  x2: number,
  y2: number,
): number {
  const dx = x2 - x1;
  const dy = y2 - y1;
  const len2 = dx * dx + dy * dy;
  if (len2 === 0) {
    const ex = px - x1;
    const ey = py - y1;
    return Math.sqrt(ex * ex + ey * ey);
  }
  let t = ((px - x1) * dx + (py - y1) * dy) / len2;
  t = Math.max(0, Math.min(1, t));
  const cx = x1 + t * dx;
  const cy = y1 + t * dy;
  const ex = px - cx;
  const ey = py - cy;
  return Math.sqrt(ex * ex + ey * ey);
}

export type { Token, Pool, Step };
