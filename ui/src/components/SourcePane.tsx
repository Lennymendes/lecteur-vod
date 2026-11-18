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
}

/** Un panneau par source. Les deux restent montés (display:none quand inactif) : la bascule est instantanée. */
export function SourcePane({ kind, slotVisible, active, busy, onPlay, error, notice, side }: Props) {
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
        <button type="submit" disabled={busy || !value.trim()}>{busy ? "Connexion…" : "Lire"}</button>
      </form>
      {error && <div className="error" role="alert">{error}</div>}
      <div className="stage">
        <VideoSlot visible={active && slotVisible}>
          {!slotVisible && <div className="placeholder">{tw ? "Saisissez une chaîne pour démarrer le direct" : "Collez un lien YouTube pour démarrer"}</div>}
        </VideoSlot>
        <div className="overlay-top">{notice}</div>
        {side}
      </div>
    </section>
  );
}
