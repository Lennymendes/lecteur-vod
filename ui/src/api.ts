// Pont IPC typé vers le cœur Rust (src-tauri/src/commands.rs, events.rs).
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type Source = "youtube" | "twitch" | "idle";

export interface AppInfo { os: string; libmpv: boolean; videoSurface: boolean; ytDlp: boolean; ffmpeg: boolean }
export interface PlayInfo { title: string; duration: number | null; route: "native" | "mpv"; video: boolean }
export interface AudioBadge {
  path: "engine" | "mpv";
  sourceRate: number; sourceChannels: number;
  deviceRate: number; deviceBits: number; deviceFloat: boolean;
  exclusive: boolean; bitPerfect: boolean; rateSwitched: boolean;
  dspInstalled: boolean; dspModifiesSignal: boolean; dspBypassed: boolean;
}
export interface Tick { position: number; duration: number | null; paused: boolean; underruns: number; title: string | null; source: Source }
export interface Notice { kind: string; text: string; from: number | null; to: number | null }

export const api = {
  info: () => invoke<AppInfo>("app_info"),
  exportLogs: () => invoke<string>("export_logs"),
  ytPlay: (url: string, start = 0) => invoke<PlayInfo>("yt_play", { url, start }),
  twitchPlay: (channel: string, quality = "best") => invoke<PlayInfo>("twitch_play", { channel, quality }),
  pause: (paused: boolean) => invoke<void>("player_pause", { paused }),
  seek: (secs: number) => invoke<void>("player_seek", { secs }),
  stop: () => invoke<void>("player_stop"),
  setAdblock: (on: boolean) => invoke<void>("set_adblock", { on }),
  setSponsorblock: (on: boolean) => invoke<void>("set_sponsorblock", { on }),
  sponsorUndo: () => invoke<void>("sponsor_undo"),
  dspLoad: (path: string) => invoke<void>("dsp_load_plugin", { path }),
  dspUnload: () => invoke<void>("dsp_unload_plugin"),
  dspBypass: (on: boolean) => invoke<void>("dsp_set_bypass", { on }),
  videoRect: (x: number, y: number, w: number, h: number, scale: number) => invoke<void>("video_set_rect", { x, y, w, h, scale }),
};

export const events = {
  audio: (f: (b: AudioBadge) => void): Promise<UnlistenFn> => listen<AudioBadge>("aurex://audio", (e) => f(e.payload)),
  tick: (f: (t: Tick) => void): Promise<UnlistenFn> => listen<Tick>("aurex://tick", (e) => f(e.payload)),
  notice: (f: (n: Notice) => void): Promise<UnlistenFn> => listen<Notice>("aurex://notice", (e) => f(e.payload)),
  error: (f: (m: string) => void): Promise<UnlistenFn> => listen<string>("aurex://error", (e) => f(e.payload)),
};

export const fmtTime = (s: number): string => {
  if (!isFinite(s) || s < 0) s = 0;
  const h = Math.floor(s / 3600), m = Math.floor((s % 3600) / 60), x = Math.floor(s % 60);
  const mm = h ? String(m).padStart(2, "0") : String(m);
  return `${h ? h + ":" : ""}${mm}:${String(x).padStart(2, "0")}`;
};
export const fmtRate = (hz: number): string => `${(hz / 1000).toLocaleString("fr-FR", { maximumFractionDigits: 1 })} kHz`;
