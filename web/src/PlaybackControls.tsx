import { useEffect } from "react";
import type { SolveResult } from "./types";

interface Props {
  result: SolveResult;
  animIndex: number;
  setAnimIndex: (v: number | ((prev: number) => number)) => void;
  playing: boolean;
  setPlaying: (v: boolean | ((prev: boolean) => boolean)) => void;
  speed: number;
  setSpeed: (v: number) => void;
}

const SPEED_MAX = 200;

// Shared playback UI: play/pause/reset/skip buttons, speed slider,
// progress bar, and the per-tick animation timer. Both Router and
// Arbitrage views replay a `trace` the same way — keeping the timer
// here ensures they stay in sync (e.g. if we change how steps per
// tick are batched).
export function PlaybackControls({
  result,
  animIndex,
  setAnimIndex,
  playing,
  setPlaying,
  speed,
  setSpeed,
}: Props) {
  useEffect(() => {
    if (!playing) return;
    const ticksPerSec = Math.min(speed, 60);
    const stepsPerTick = Math.max(1, Math.round(speed / ticksPerSec));
    const intervalMs = 1000 / ticksPerSec;
    const id = window.setInterval(() => {
      setAnimIndex((i) => {
        const next = Math.min(result.trace.length, i + stepsPerTick);
        if (next >= result.trace.length) setPlaying(false);
        return next;
      });
    }, intervalMs);
    return () => window.clearInterval(id);
  }, [playing, result, speed, setAnimIndex, setPlaying]);

  return (
    <>
      <div className="playback-row">
        <button
          className="btn"
          onClick={() => setPlaying((p) => !p)}
          disabled={animIndex >= result.trace.length}
          title={playing ? "Pause" : "Play"}
        >
          {playing ? "⏸" : "▶"}
        </button>
        <button
          className="btn"
          onClick={() => {
            setPlaying(false);
            setAnimIndex(0);
          }}
          title="Reset"
        >
          ⟲
        </button>
        <button
          className="btn"
          onClick={() => {
            setPlaying(false);
            setAnimIndex(result.trace.length);
          }}
          title="Skip to end"
        >
          ⏭
        </button>
      </div>
      <label>
        <span>Speed</span>
        <span className="mono small">{speed}/s</span>
      </label>
      <input
        type="range"
        min={1}
        max={SPEED_MAX}
        value={speed}
        onChange={(e) => setSpeed(+e.target.value)}
      />
      <div className="progress">
        <div
          className="progress-fill"
          style={{
            width: `${(animIndex / Math.max(1, result.trace.length)) * 100}%`,
          }}
        />
      </div>
    </>
  );
}
