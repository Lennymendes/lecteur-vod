//! macOS : NSView ajoutée SOUS toutes les sous-vues de la vue contenu (donc sous WKWebView).
//! La fenêtre est transparente (`macOSPrivateApi`), la webview a un fond transparent.
//! ⚠ À valider (Spike 1) sur Apple Silicon et Intel. libmpv (`wid` = NSView*) ajoute sa propre
//! sous-vue DANS notre conteneur : l'ordre Z global reste « vidéo sous webview ».

use objc2::encode::{Encoding, RefEncode};
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::msg_send;
use objc2_app_kit::{NSColor, NSView, NSWindowOrderingMode};
use objc2_foundation::{MainThreadMarker, NSPoint, NSRect, NSSize};
use raw_window_handle::RawWindowHandle;

#[repr(C)]
struct CGColor {
    _p: [u8; 0],
}
unsafe impl RefEncode for CGColor {
    const ENCODING_REF: Encoding = Encoding::Pointer(&Encoding::Struct("CGColor", &[]));
}

static CALLS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

pub struct Surface {
    view: usize,
    content: usize,
}

// SAFETY : les pointeurs ne sont déréférencés que sur le thread principal (voir commands::video_set_rect).
unsafe impl Send for Surface {}
unsafe impl Sync for Surface {}

impl Surface {
    pub fn create(parent: RawWindowHandle) -> Result<Self, String> {
        let RawWindowHandle::AppKit(h) = parent else { return Err("handle de fenêtre non AppKit".into()) };
        unsafe {
            let content: &NSView = &*(h.ns_view.as_ptr() as *const NSView);
            let mtm = MainThreadMarker::new_unchecked(); // appelé depuis `setup` (thread principal)
            let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(16.0, 16.0));
            let view: Retained<NSView> = NSView::initWithFrame(mtm.alloc::<NSView>(), frame);
            // Fond noir opaque : les bandes autour de la vidéo (letterbox) ne laissent plus voir le bureau.
            view.setWantsLayer(true);
            let layer: *mut AnyObject = msg_send![&*view, layer];
            if !layer.is_null() {
                let black = NSColor::blackColor();
                let cg: *const CGColor = msg_send![&*black, CGColor];
                let _: () = msg_send![layer, setBackgroundColor: cg];
            }
            content.addSubview_positioned_relativeTo(&view, NSWindowOrderingMode::NSWindowBelow, None);
            let f = content.frame();
            crate::log::log(format!("mac: vue contenu {}x{}, sous-vues={}", f.size.width, f.size.height, content.subviews().len()));
            Ok(Surface { view: Retained::into_raw(view) as usize, content: h.ns_view.as_ptr() as usize })
        }
    }

    pub fn wid(&self) -> i64 {
        self.view as i64
    }

    /// Pixels CSS = points sur macOS ; `_scale` inutilisé. Repère AppKit : origine en bas à gauche.
    pub fn set_rect(&self, x: f64, y: f64, w: f64, h: f64, _scale: f64) {
        unsafe {
            let content: &NSView = &*(self.content as *const NSView);
            let view: &NSView = &*(self.view as *const NSView);
            let ch = content.frame().size.height;
            view.setFrame(NSRect::new(NSPoint::new(x, ch - y - h), NSSize::new(w.max(1.0), h.max(1.0))));
            view.setHidden(w < 2.0 || h < 2.0);
            let n = CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if n < 15 || n % 50 == 0 {
                crate::log::log(format!("mac: set_rect #{n} css=({x:.0},{y:.0},{w:.0},{h:.0}) contenu_h={ch:.0} masquée={} sous-vues vue={}", w < 2.0 || h < 2.0, view.subviews().len()));
            }
        }
    }
}
