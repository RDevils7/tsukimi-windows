//! No-op MPRIS stubs for platforms without a D-Bus session bus
//! (Windows, macOS).
//!
//! The real implementation lives in the sibling `mpris` module and is
//! compiled on Linux only: it registers the music player on the session bus
//! via the `mpris-server` crate. Everywhere else there is no session bus to
//! register with, so every method here is a no-op. The signatures are kept
//! identical to the Linux ones so that call sites need no platform gates.

use anyhow::Result;

use super::player::MusicPlayer;
use crate::gstl::player::imp::ListRepeatMode;

impl MusicPlayer {
    pub async fn initialize_mpris(&self) -> Result<()> {
        Ok(())
    }

    pub fn notify_mpris_song_changed(&self, _has_prev: bool, _has_next: bool) {}

    pub fn notify_mpris_playing(&self) {}

    pub fn notify_mpris_paused(&self) {}

    pub fn notify_mpris_stopped(&self) {}

    pub fn notify_mpris_seeked(&self, _position: i64) {}

    pub fn notify_mpris_loop_status(&self, _status: ListRepeatMode) {}
}
