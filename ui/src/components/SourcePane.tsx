import { FormEvent, useState } from "react";
import { VideoSlot } from "./VideoSlot";

interface Props {
  kind: "twitch" | "youtube";
  /** Le slot n'est affiché que si cet onglet est actif ET que c'est lui qui joue. */
  slotVisible: boolean;
  active: boolean;
  busy: boolean;
  onPlay: (value: string) => void;
  error: string | null;
  notice: React.ReactNode;
  side?: React.ReactNode;
  quality: string;
  onQuality: (q: string) => void;
  placeholder?: string;
}

const TW_Q = [["best", "Source"], ["1080p", "1080p"], ["720p60", "720p60"], ["720p", "720p"], ["480p", "480p"], ["360p", "360p"], ["160p", "160p"], ["audio_only", "Audio seul"]];
const YT_Q = [["2160", "2160p (4K)"], ["1440", "1440p"], ["1080", "1080p"], ["720", "720p"], ["480", "480p"], ["360", "360p"]];

/** Un panneau par source. Les deux restent montés (display:none quand inactif) : la bascule est instantanée. */
export function SourcePane({ kind, slotVisible, active, busy, onPlay, error, notice, side, quality, onQuality, placeholder }: Props) {
  const [value, setValue] = useState("");
  const submit = (e: FormEvent) => { e.preventDefault(); if (value.trim()) onPlay(value.trim()); };

  const tw = kind === "twitch";
  return (
    <section className="pane" hidden={!active} aria-hidden={!active}>
      <form className="omni" onSubmit={submit}>
        <input
          value={value} onChange={(e) => setValue(e.target.value)} spellCheck={false} autoCapitalize="off"
          placeholder={tw ? "Nom de la chaîne Twitch (ex. gotaga)" : "URL ou identifiant YouTube"}
          aria-label={tw ? "Chaîne Twitch" : "Vidéo YouTube"}
        />
        <select className="quality" value={quality} onChange={(e) => onQuality(e.target.value)} aria-label="Qualité" title={tw ? "Qualité du flux" : "Résolution maximale"}>
          {(tw ? TW_Q : YT_Q).map(([v, l]) => <option key={v} value={v}>{l}</option>)}
        </select>
        <button type="submit" disabled={busy || !value.trim()}>{busy ? "Connexion…" : "Lire"}</button>
      </form>
      {error && <div className="error" role="alert">{error}</div>}
      <div className="stage">
        <VideoSlot visible={active && slotVisible}>
          {!slotVisible && <div className="placeholder">{placeholder ?? (tw ? "Saisissez une chaîne pour démarrer le direct" : "Collez un lien YouTube pour démarrer")}</div>}
        </VideoSlot>
        <div className="overlay-top">{notice}</div>
        {side}
      </div>
    </section>
  );
}
