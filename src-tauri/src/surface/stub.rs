//! Linux / autres : pas de surface (développement de l'UI uniquement). libmpv ouvrira sa propre fenêtre.
use raw_window_handle::RawWindowHandle;

pub struct Surface;

impl Surface {
    pub fn create(_parent: RawWindowHandle) -> Result<Self, String> {
        Ok(Surface)
    }
    pub fn wid(&self) -> i64 {
        0
    }
    pub fn set_rect(&self, _x: f64, _y: f64, _w: f64, _h: f64, _scale: f64) {}
}
