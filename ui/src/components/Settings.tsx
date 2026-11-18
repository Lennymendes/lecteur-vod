import { useEffect, useState } from "react";
import { api, Player, Settings } from "../api";

const PLAYERS: { id: Player; label: string; hint: string; mac?: boolean }[] = [
  { id: "aurex", label: "Aurex (intégré)", hint: "Audio exclusif bit-perfect, pont DSP, SponsorBlock." },
  { id: "vlc", label: "VLC", hint: "Doit être installé. Twitch passe par le proxy anti-pub d'Aurex." },
  { id: "mpv", label: "mpv", hint: "Doit être installé (Homebrew, scoop…)." },
  { id: "iina", label: "IINA", hint: "macOS uniquement.", mac: true },
  { id: "custom", label: "Commande personnalisée", hint: "Jetons : {video} {audio} {title}" },
];

export function SettingsPanel({ s, onChange, onClose, os, dspInstalled }: {
  s: Settings; onChange: (s: Settings) => void; onClose: () => void; os: string; dspInstalled: boolean;
}) {
  const [plugin, setPlugin] = useState("");
  const [msg, setMsg] = useState<string | null>(null);
  useEffect(() => {
    const esc = (e: KeyboardEvent) => { if (e.key === "Escape") onClose(); };
    window.addEventListener("keydown", esc);
    return () => window.removeEventListener("keydown", esc);
  }, [onClose]);

  return (
    <div className="modal-backdrop" onMouseDown={(e) => { if (e.target === e.currentTarget) onClose(); }}>
      <div className="modal" role="dialog" aria-label="Réglages">
        <header><h2>Réglages</h2><button className="ghost" onClick={onClose} aria-label="Fermer">✕</button></header>

        <section>
          <h3>Lecteur</h3>
          <div className="players">
            {PLAYERS.filter((p) => !p.mac || os === "macos").map((p) => (
              <label key={p.id} className={`player ${s.player === p.id ? "on" : ""}`}>
                <input type="radio" name="player" checked={s.player === p.id} onChange={() => onChange({ ...s, player: p.id })} />
                <span><strong>{p.label}</strong><small>{p.hint}</small></span>
              </label>
            ))}
          </div>
          {s.player === "custom" && (
            <input className="field" value={s.customCmd} spellCheck={false} placeholder="vlc {video} --input-slave={audio}"
              onChange={(e) => onChange({ ...s, customCmd: e.target.value })} />
          )}
          {s.player !== "aurex" && <p className="hint">Avec un lecteur externe, l'audio exclusif et le pont DSP d'Aurex ne s'appliquent pas.</p>}
        </section>

        <section>
          <h3>Plugin DSP (C/C++)</h3>
          <div className="row">
            <input className="field" value={plugin} onChange={(e) => setPlugin(e.target.value)} placeholder="Chemin de la bibliothèque (.dylib / .dll)" spellCheck={false} />
            <button className="primary" onClick={() => api.dspLoad(plugin).then(() => setMsg("Chargé : actif au prochain titre")).catch((e) => setMsg(String(e)))}>Charger</button>
            <button className="ghost" disabled={!dspInstalled} onClick={() => api.dspUnload().then(() => setMsg("Déchargé")).catch((e) => setMsg(String(e)))}>Retirer</button>
          </div>
        </section>

        <section>
          <h3>Diagnostic</h3>
          <button className="ghost" onClick={() => api.exportLogs().then((p) => setMsg(`Journaux exportés : ${p}`)).catch((e) => setMsg(String(e)))}>Exporter les journaux sur le Bureau</button>
        </section>
        {msg && <p className="hint selectable">{msg}</p>}
      </div>
    </div>
  );
}
