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

/** Interrupteurs actifs par défaut : anti-pub Twitch et SponsorBlock sont ON au démarrage. */
export function Toggles() {
  const [ad, setAd] = useState(true);
  const [sb, setSb] = useState(true);
  const [bypass, setBypass] = useState(false);
  return (
    <div className="toggles">
      <Switch label="Anti-pub Twitch" hint="Contourne les pubs SSAI (playlist HLS nettoyée en local)" on={ad} onChange={(v) => { setAd(v); api.setAdblock(v).catch(() => {}); }} />
      <Switch label="SponsorBlock" hint="Saute les segments sponsor / intro / outro (YouTube)" on={sb} onChange={(v) => { setSb(v); api.setSponsorblock(v).catch(() => {}); }} />
      <Switch label="Contourner le DSP" hint="Laisse passer le signal sans traitement (bit-perfect)" on={bypass} onChange={(v) => { setBypass(v); api.dspBypass(v).catch(() => {}); }} />
    </div>
  );
}
