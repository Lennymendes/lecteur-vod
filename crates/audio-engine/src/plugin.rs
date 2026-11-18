//! Chargement d'un plugin DSP natif (C/C++/Rust) et exports C pour un plugin lié statiquement.
//!
//! Plugin dynamique : une bibliothèque (.dylib / .dll) exportant
//! `const AurexDspApi* aurex_dsp_entry(void)` (voir `include/aurex_dsp.h`).
//! L'hôte copie la table de fonctions ; la bibliothèque reste chargée jusqu'à `unload_plugin`.

use audio_core::dsp::{AurexDspApi, DspBridge, DspError, AUREX_DSP_ABI_VERSION};
use std::path::Path;

pub type EntryFn = unsafe extern "C" fn() -> *const AurexDspApi;

/// Valide la table (version, taille) AVANT de la copier en entier.
unsafe fn read_api(p: *const AurexDspApi) -> Result<AurexDspApi, DspError> {
    if p.is_null() {
        return Err(DspError::Load("table nulle".into()));
    }
    let version = std::ptr::read(std::ptr::addr_of!((*p).abi_version));
    let size = std::ptr::read(std::ptr::addr_of!((*p).struct_size));
    if version != AUREX_DSP_ABI_VERSION {
        return Err(DspError::AbiMismatch { host: AUREX_DSP_ABI_VERSION, plugin: version });
    }
    if (size as usize) < std::mem::size_of::<AurexDspApi>() {
        return Err(DspError::BadStruct);
    }
    Ok(std::ptr::read(p))
}

/// Charge `path`, installe le plugin dans le pont global. Thread de contrôle uniquement.
pub fn load_plugin(path: &Path) -> Result<(), DspError> {
    unsafe {
        let lib = libloading::Library::new(path).map_err(|e| DspError::Load(e.to_string()))?;
        let api = {
            let entry: libloading::Symbol<EntryFn> =
                lib.get(b"aurex_dsp_entry\0").map_err(|e| DspError::Load(format!("aurex_dsp_entry introuvable : {e}")))?;
            read_api(entry())?
        };
        DspBridge::global().install(api, Some(Box::new(lib)))
    }
}

pub fn unload_plugin() {
    DspBridge::global().uninstall();
}

// ───── Exports C : pour un plugin compilé DANS le même binaire (liaison statique) ─────

/// # Safety
/// `api` doit pointer vers une `AurexDspApi` valide pendant l'appel.
#[no_mangle]
pub unsafe extern "C" fn aurex_dsp_install(api: *const AurexDspApi) -> i32 {
    match read_api(api).and_then(|a| DspBridge::global().install(a, None)) {
        Ok(()) => 0,
        Err(DspError::AbiMismatch { .. }) => -2,
        Err(DspError::BadStruct) => -3,
        Err(DspError::NoCallbacks) => -4,
        Err(_) => -1,
    }
}

#[no_mangle]
pub extern "C" fn aurex_dsp_uninstall() {
    DspBridge::global().uninstall();
}

#[no_mangle]
pub extern "C" fn aurex_dsp_set_bypass(on: i32) {
    DspBridge::global().set_bypass(on != 0);
}
