import { useState } from "react";
import { api } from "../api";

function Switch({ label, hint, on, onChange }: { label: string; hint?: string; on: boolean; onChange: (v: boolean) => void }) {
  return (
    <label className="switch" title={hint}>
      <input type="checkbox" checked={on} onChange={(e) => onChange(e.target.checked)} />
      <span className="track"><span className="thumb" /></span>
      <span className="lbl">{label}</span>
    </label>
  );
}

export function Toggles({ dspInstalled }: { dspInstalled: boolean }) {
  const [ad, setAd] = useState(true);
  const [sb, setSb] = useState(true);
  const [bypass, setBypass] = useState(false);
  const [plugin, setPlugin] = useState("");
  const [msg, setMsg] = useState<string | null>(null);
  return (
    <div className="toggles">
      <Switch label="Anti-pub Twitch" hint="Contourne les pubs SSAI (playlist HLS nettoyée en local)" on={ad} onChange={(v) => { setAd(v); api.setAdblock(v).catch(() => {}); }} />
      <Switch label="SponsorBlock" hint="Saute les segments sponsor / intro / outro (YouTube)" on={sb} onChange={(v) => { setSb(v); api.setSponsorblock(v).catch(() => {}); }} />
      <Switch label="Contourner le DSP" hint="Laisse passer le signal sans traitement (bit-perfect)" on={bypass} onChange={(v) => { setBypass(v); api.dspBypass(v).catch(() => {}); }} />
      <details className="dsp">
        <summary>Plugin DSP (C/C++)</summary>
        <div className="dsp-row">
          <input value={plugin} onChange={(e) => setPlugin(e.target.value)} placeholder="Chemin de la bibliothèque (.dll / .dylib)" spellCheck={false} />
          <button onClick={() => api.dspLoad(plugin).then(() => setMsg("Chargé — actif au prochain titre")).catch((e) => setMsg(String(e)))}>Charger</button>
          <button className="ghost" disabled={!dspInstalled} onClick={() => api.dspUnload().then(() => setMsg("Déchargé")).catch((e) => setMsg(String(e)))}>Retirer</button>
        </div>
        {msg && <div className="hint">{msg}</div>}
      </details>
    </div>
  );
}
