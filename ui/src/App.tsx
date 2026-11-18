import { useCallback, useEffect, useRef, useState } from "react";
import { api, AppInfo, AudioBadge, events, externalOf, fmtTime, loadSettings, Notice, PlayInfo, saveSettings, Settings, Source, Tick } from "./api";
import { SettingsPanel } from "./components/Settings";
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
  const [settings, setSettingsState] = useState<Settings>(loadSettings);
  const [showSettings, setShowSettings] = useState(false);
  const last = useRef<Record<Tab, string | null>>({ twitch: null, youtube: null });
  const setSettings = (s: Settings) => { setSettingsState(s); saveSettings(s); };

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

  const play = useCallback(async (t: Tab, value: string, s: Settings) => {
    last.current[t] = value;
    setBusy(t);
    setErrors((e) => ({ ...e, [t]: null }));
    setBadge(null);
    try {
      const ext = externalOf(s);
      const r = t === "twitch" ? await api.twitchPlay(value, s.twitchQuality, ext) : await api.ytPlay(value, s.ytMaxHeight, ext);
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

  // Changer la qualité relance le flux en cours de cette source.
  const changeQuality = (t: Tab, q: string) => {
    const s = t === "twitch" ? { ...settings, twitchQuality: q } : { ...settings, ytMaxHeight: Number(q) };
    setSettings(s);
    if (playing === t && last.current[t]) play(t, last.current[t]!, s);
  };
  const extPlaceholder = (t: Tab) => (now?.route === "external" && last.current[t] && now.title ? `▶ ${now.title}` : undefined);

  const missing = info && (!info.libmpv || !info.ytDlp || !info.ffmpeg);

  return (
    <div className="app">
      <TitleBar tab={tab} onTab={setTab} playing={playing} os={info?.os ?? ""} onSettings={() => setShowSettings(true)} />
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
          onPlay={(v) => play("twitch", v, settings)} error={errors.twitch} notice={noticeNode}
          quality={settings.twitchQuality} onQuality={(q) => changeQuality("twitch", q)} placeholder={extPlaceholder("twitch")}
        />
        <SourcePane
          kind="youtube" active={tab === "youtube"} slotVisible={playing === "youtube" && now?.video !== false} busy={busy === "youtube"}
          onPlay={(v) => play("youtube", v, settings)} error={errors.youtube} notice={noticeNode}
          quality={String(settings.ytMaxHeight)} onQuality={(q) => changeQuality("youtube", q)} placeholder={extPlaceholder("youtube")}
        />
      </main>
      <footer className="dock">
        <div className="now">
          <div className="title">{tick?.title ?? (now?.route === "external" ? now.title : "Rien en lecture")}</div>
          <StatusBadge b={badge} underruns={tick?.underruns ?? 0} />
        </div>
        <Controls tick={tick} />
        <Toggles />
      </footer>
      {showSettings && (
        <SettingsPanel s={settings} onChange={setSettings} onClose={() => setShowSettings(false)} os={info?.os ?? ""} dspInstalled={badge?.dspInstalled ?? false} />
      )}
    </div>
  );
}
