import { getCurrentWindow } from "@tauri-apps/api/window";

export type Tab = "twitch" | "youtube";

export function TitleBar({ tab, onTab, playing, os, onSettings }: { tab: Tab; onTab: (t: Tab) => void; playing: "twitch" | "youtube" | "idle"; os: string; onSettings: () => void }) {
  const win = getCurrentWindow();
  const mac = os === "macos";
  return (
    <header className="titlebar" data-tauri-drag-region style={{ paddingLeft: mac ? 84 : 14 }}>
      <div className="brand" data-tauri-drag-region>Aurex</div>
      <nav className="tabs" role="tablist" aria-label="Source">
        {(["twitch", "youtube"] as Tab[]).map((t) => (
          <button key={t} role="tab" aria-selected={tab === t} className={`tab ${tab === t ? "on" : ""}`} onClick={() => onTab(t)}>
            <span className={`dot ${t}`} />
            {t === "twitch" ? "Twitch" : "YouTube"}
            {playing === t && <span className="live-pip" title="En lecture" />}
          </button>
        ))}
      </nav>
      <div className="spacer" data-tauri-drag-region />
      <button className="gear" onClick={onSettings} aria-label="Réglages" title="Réglages">⚙︎</button>
      {!mac && (
        <div className="winctl">
          <button aria-label="Réduire" onClick={() => win.minimize()}>–</button>
          <button aria-label="Agrandir" onClick={() => win.toggleMaximize()}>▢</button>
          <button aria-label="Fermer" className="close" onClick={() => win.close()}>✕</button>
        </div>
      )}
    </header>
  );
}
