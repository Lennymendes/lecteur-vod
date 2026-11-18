import { AudioBadge, fmtRate } from "../api";

export function StatusBadge({ b, underruns }: { b: AudioBadge | null; underruns: number }) {
  if (!b) return <div className="badge idle">Audio : en attente</div>;
  const fmt = b.deviceFloat ? "float 32" : b.deviceBits ? `${b.deviceBits} bits` : "format ?";
  const cls = b.path === "engine" ? (b.bitPerfect ? "ok" : "warn") : "info";
  return (
    <div className={`badge ${cls}`} title={`Source ${fmtRate(b.sourceRate)} · ${b.sourceChannels} canaux`}>
      <strong>{b.exclusive ? "Exclusif" : "Partagé"}</strong>
      <span>{fmt} · {fmtRate(b.deviceRate)}</span>
      {b.rateSwitched && <span className="chip">fréquence commutée</span>}
      {b.path === "engine" && b.bitPerfect && <span className="chip good">bit-perfect</span>}
      {b.path === "engine" && !b.bitPerfect && <span className="chip warn">{b.dspModifiesSignal && !b.dspBypassed ? "DSP actif" : "non bit-perfect"}</span>}
      {b.path === "mpv" && <span className="chip">sortie mpv · hors pont DSP</span>}
      {b.dspInstalled && <span className="chip">{b.dspBypassed ? "DSP contourné" : "DSP"}</span>}
      {underruns > 0 && <span className="chip warn">{underruns} underruns</span>}
    </div>
  );
}
