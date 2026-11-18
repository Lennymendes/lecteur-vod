import { useState } from "react";
import { api, fmtTime, Tick } from "../api";

export function Controls({ tick }: { tick: Tick | null }) {
  const [drag, setDrag] = useState<number | null>(null);
  const idle = !tick || tick.source === "idle";
  const dur = tick?.duration ?? null;
  const pos = drag ?? tick?.position ?? 0;
  return (
    <div className="controls">
      <button className="round" disabled={idle} onClick={() => api.pause(!tick!.paused)} aria-label={tick?.paused ? "Lecture" : "Pause"}>
        {tick?.paused ? "▶" : "❚❚"}
      </button>
      <span className="time">{fmtTime(pos)}</span>
      {dur ? (
        <input
          className="seek" type="range" min={0} max={dur} step={0.5} value={Math.min(pos, dur)} disabled={idle}
          onChange={(e) => setDrag(Number(e.target.value))}
          onPointerUp={() => { if (drag !== null) { api.seek(drag).catch(() => {}); setDrag(null); } }}
          onKeyUp={() => { if (drag !== null) { api.seek(drag).catch(() => {}); setDrag(null); } }}
        />
      ) : (
        <div className="seek live">{idle ? "" : "DIRECT"}</div>
      )}
      <span className="time">{dur ? fmtTime(dur) : ""}</span>
      <button className="ghost" disabled={idle} onClick={() => api.stop()} aria-label="Arrêter">■</button>
    </div>
  );
}
