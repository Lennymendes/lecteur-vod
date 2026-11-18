use crate::{AudioError, ExclusiveOutput};

#[cfg(target_os = "macos")]
mod coreaudio;
#[cfg(target_os = "windows")]
mod wasapi_excl;
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod null;

/// Backend exclusif de la plateforme (périphérique de sortie par défaut).
pub fn default_output() -> Result<Box<dyn ExclusiveOutput>, AudioError> {
    #[cfg(target_os = "macos")]
    return Ok(Box::new(coreaudio::CoreAudioOutput::new()));
    #[cfg(target_os = "windows")]
    return Ok(Box::new(wasapi_excl::WasapiExclusive::new()));
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    return Ok(Box::new(null::NullOutput::new())); // dev/CI Linux : consomme au rythme temps réel
}
