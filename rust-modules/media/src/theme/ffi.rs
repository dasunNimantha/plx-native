//! theme::ffi — the seam's theme verbs (`sf_theme_*`, `src/starfish.h`). Signatures mirror the
//! header exactly. Not compiled for `hostsim` or this crate's tests, like `player::ffi`.
//!
//! The seam keeps no lock for the theme object: every verb but `load` must be called from the one
//! thread that owns the theme pipeline, and only after `load` returned on it.
use std::os::raw::{c_char, c_int, c_uint};

extern "C" {
    pub(crate) fn sf_theme_load(payload: *const c_char, epoch: c_uint) -> c_int;
    pub(crate) fn sf_theme_play() -> c_int;
    pub(crate) fn sf_theme_set_volume(volume: c_int) -> c_int;
    pub(crate) fn sf_theme_feed(p: *const u8, size: c_uint, pts: i64) -> c_char;
    pub(crate) fn sf_theme_push_eos() -> c_int;
    pub(crate) fn sf_theme_unload();
    pub(crate) fn sf_theme_unload_completed() -> c_int;
    pub(crate) fn sf_theme_release() -> c_int;
}
