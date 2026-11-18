import { useEffect, useRef } from "react";
import { api } from "../api";

/**
 * « Trou » transparent dans l'UI, sous lequel la surface native (libmpv) est positionnée.
 * Envoie son rectangle au cœur Rust à chaque changement. `visible=false` → rectangle nul (surface masquée).
 */
export function VideoSlot({ visible, children }: { visible: boolean; children?: React.ReactNode }) {
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    let raf = 0;
    const push = () => {
      raf = 0;
      if (!visible) return void api.videoRect(0, 0, 0, 0, 1).catch(() => {});
      const r = el.getBoundingClientRect();
      api.videoRect(r.left, r.top, r.width, r.height, window.devicePixelRatio || 1).catch(() => {});
    };
    const schedule = () => { if (!raf) raf = requestAnimationFrame(push); };
    schedule();
    const ro = new ResizeObserver(schedule);
    ro.observe(el);
    window.addEventListener("resize", schedule);
    // Les panneaux latéraux qui s'ouvrent/ferment déplacent le slot sans le redimensionner.
    const mo = new MutationObserver(schedule);
    mo.observe(document.body, { attributes: true, subtree: true, attributeFilter: ["class", "style"] });
    return () => {
      ro.disconnect(); mo.disconnect();
      window.removeEventListener("resize", schedule);
      if (raf) cancelAnimationFrame(raf);
    };
  }, [visible]);

  return <div ref={ref} className="video-slot">{children}</div>;
}
