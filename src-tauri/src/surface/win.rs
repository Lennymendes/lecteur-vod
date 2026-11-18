//! Windows : fenêtre enfant (HWND) au fond de la pile Z de la fenêtre principale, sous WebView2.
//! ⚠ À valider (Spike 1) : WebView2 doit rester transparent au-dessus d'un HWND frère. Si ce n'est pas
//! le cas sur certaines configs GPU, repli prévu : fenêtre vidéo séparée + fenêtre UI transparente
//! au-dessus (voir docs/SPIKE1.md).

use raw_window_handle::RawWindowHandle;
use std::ffi::c_void;
use windows::core::w;
use windows::Win32::Foundation::{HINSTANCE, HWND};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, SetWindowPos, HMENU, HWND_BOTTOM, SWP_HIDEWINDOW, SWP_NOACTIVATE, SWP_SHOWWINDOW, WINDOW_EX_STYLE, WS_CHILD, WS_CLIPCHILDREN,
    WS_CLIPSIBLINGS, WS_VISIBLE,
};

pub struct Surface {
    hwnd: isize,
}

impl Surface {
    pub fn create(parent: RawWindowHandle) -> Result<Self, String> {
        let RawWindowHandle::Win32(h) = parent else { return Err("handle de fenêtre non Win32".into()) };
        let parent = HWND(h.hwnd.get() as *mut c_void);
        let child = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("STATIC"),
                w!(""),
                WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS | WS_CLIPCHILDREN,
                0,
                0,
                16,
                16,
                parent,
                HMENU::default(),
                HINSTANCE::default(),
                None,
            )
        }
        .map_err(|e| format!("CreateWindowExW : {e}"))?;
        unsafe { SetWindowPos(child, HWND_BOTTOM, 0, 0, 16, 16, SWP_NOACTIVATE) }.map_err(|e| format!("SetWindowPos : {e}"))?;
        Ok(Surface { hwnd: child.0 as isize })
    }

    /// Valeur à passer à `mpv --wid`.
    pub fn wid(&self) -> i64 {
        self.hwnd as i64
    }

    /// Rectangle en pixels CSS relatifs à la zone client ; `scale` = devicePixelRatio.
    pub fn set_rect(&self, x: f64, y: f64, w: f64, h: f64, scale: f64) {
        let hwnd = HWND(self.hwnd as *mut c_void);
        let visible = w >= 2.0 && h >= 2.0;
        let flags = SWP_NOACTIVATE | if visible { SWP_SHOWWINDOW } else { SWP_HIDEWINDOW };
        let _ = unsafe { SetWindowPos(hwnd, HWND_BOTTOM, (x * scale) as i32, (y * scale) as i32, (w * scale).max(1.0) as i32, (h * scale).max(1.0) as i32, flags) };
    }
}
