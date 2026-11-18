//! Lecteur vidéo : libmpv (feature `libmpv`) ou stub.

#[cfg(feature = "libmpv")]
mod mpv;
#[cfg(not(feature = "libmpv"))]
mod null;

#[cfg(feature = "libmpv")]
pub use mpv::Video;
#[cfg(not(feature = "libmpv"))]
pub use null::Video;
