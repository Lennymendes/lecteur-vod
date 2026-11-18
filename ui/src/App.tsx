import { useCallback, useEffect, useRef, useState } from "react";
import { api, AppInfo, AudioBadge, events, fmtTime, Notice, PlayInfo, Source, Tick } from "./api";
import { Controls } from "./components/Controls";
import { SourcePane } from "./components/SourcePane";
import { StatusBadge } from "./components/StatusBadge";
import { Tab, TitleBar } from "./components/TitleBar";
import { Toggles } from "./components/Toggles";

const NOTICE_TEXT: Record<string, (n: Notice) => string> = {
  "sponsor-skip": (n) => `Segment « ${n.text} » sauté (${fmtTime(n.from ?? 0)} → ${fmtTime(n.to ?? 0)})`,
  "ad-backup": (n) => `Pub contournée via un flux de secours (${n.text})`,
  "ad-stripped": (n) => `Coupure pub ${n.text} — retour automatique`,
  "ad-clean": () => "Flux normal rétabli",
  "video-resync": (n) => `Vidéo recalée (${n.text})`,
  ended: () => "Lecture terminée",
};

export default function App() {
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [tab, setTab] = useState<Tab>("youtube");
  const [tick, setTick] = useState<Tick | null>(null);
  const [badge, setBadge] = useState<AudioBadge | null>(null);
  const [now, setNow] = useState<PlayInfo | null>(null);
  const [busy, setBusy] = useState<Tab | null>(null);
  const [errors, setErrors] = useState<Record<Tab, string | null>>({ twitch: null, youtube: null });
  const [notice, setNotice] = useState<Notice | null>(null);
  const noticeTimer = useRef<number>();

  useEffect(() => { api.info().then(setInfo).catch(() => {}); }, []);

  useEffect(() => {
    const subs = [
      events.tick((t) => setTick(t)),
      events.audio((b) => setBadge(b)),
      events.notice((n) => {
        setNotice(n);
        window.clearTimeout(noticeTimer.current);
        noticeTimer.current = window.setTimeout(() => setNotice(null), n.kind === "sponsor-skip" ? 6000 : 4000);
        if (n.kind === "ended") setTick(null);
      }),
      events.error((m) => setErrors((e) => ({ ...e, [tab]: m }))),
    ];
    return () => { subs.forEach((p) => p.then((un) => un())); };
  }, [tab]);

  const playing: Source = tick?.source ?? "idle";

  const play = useCallback(async (t: Tab, value: string) => {
    setBusy(t);
    setErrors((e) => ({ ...e, [t]: null }));
    setBadge(null);
    try {
      const r = t === "twitch" ? await api.twitchPlay(value) : await api.ytPlay(value);
      setNow(r);
    } catch (e) {
      setErrors((x) => ({ ...x, [t]: String(e) }));
    } finally {
      setBusy(null);
    }
  }, []);

  const noticeNode = notice && (
    <div className={`toast ${notice.kind}`}>
      {(NOTICE_TEXT[notice.kind] ?? ((n: Notice) => n.text))(notice)}
      {notice.kind === "sponsor-skip" && <button onClick={() => api.sponsorUndo()}>Annuler</button>}
    </div>
  );

  const missing = info && (!info.libmpv || !info.ytDlp || !info.ffmpeg);

  return (
    <div className="app">
      <TitleBar tab={tab} onTab={setTab} playing={playing} os={info?.os ?? ""} />
      {missing && (
        <div className="banner">
          {!info!.libmpv && "libmpv non compilé : pas de vidéo. "}
          {!info!.ytDlp && "Sidecar yt-dlp absent (scripts/fetch-sidecars). "}
          {!info!.ffmpeg && "Sidecar ffmpeg absent. "}
        </div>
      )}
      <main className="main">
        <SourcePane
          kind="twitch" active={tab === "twitch"} slotVisible={playing === "twitch"} busy={busy === "twitch"}
          onPlay={(v) => play("twitch", v)} error={errors.twitch} notice={noticeNode}
        />
        <SourcePane
          kind="youtube" active={tab === "youtube"} slotVisible={playing === "youtube" && now?.video !== false} busy={busy === "youtube"}
          onPlay={(v) => play("youtube", v)} error={errors.youtube} notice={noticeNode}
        />
      </main>
      <footer className="dock">
        <div className="now">
          <div className="title">{tick?.title ?? "Rien en lecture"}</div>
          <StatusBadge b={badge} underruns={tick?.underruns ?? 0} />
        </div>
        <Controls tick={tick} />
        <Toggles dspInstalled={badge?.dspInstalled ?? false} />
      </footer>
    </div>
  );
}
