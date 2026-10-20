//! No-op MPRIS stubs for platforms without a D-Bus session bus
//! (Windows, macOS).
//!
//! The real implementation lives in the sibling `mpris` module and is
//! compiled on Linux only: it registers the player on the session bus via
//! the `mpris-server` crate so desktop environments and media keys can
//! control playback. Everywhere else there is no session bus to register
//! with, so every method here is a no-op. The signatures are kept identical
//! to the Linux ones so that call sites need no platform gates.

use anyhow::Result;

use crate::{
    gstl::player::imp::ListRepeatMode,
    ui::mpv::page::MPVPage,
};

impl MPVPage {
    pub async fn initialize_mpris(&self, _app_id: &str) -> Result<()> {
        Ok(())
    }

    pub fn notify_mpris_seeked(&self, _position: i64) {}

    pub fn notify_mpris_track_changed(&self) {}

    pub fn notify_mpris_playing(&self) {}

    pub fn notify_mpris_paused(&self) {}

    pub fn notify_mpris_volume(&self, _volume: f64) {}

    pub fn notify_mpris_stopped(&self) {}

    pub fn notify_mpris_loop_status(&self, _status: ListRepeatMode) {}

    /// Whether the track list differs from the one MPRIS was last told about.
    /// Always `false` without MPRIS: nothing consumes the track list.
    pub fn mpris_track_list_changed(&self, _episode_list: &[crate::ui::provider::tu_item::TuItem]) -> bool {
        false
    }

    pub(crate) fn notify_mpris_track_list_replaced(&self) {}
}
