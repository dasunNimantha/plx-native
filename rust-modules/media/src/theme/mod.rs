//! theme — theme music: a show's theme song, played quietly while its detail page is open.
//!
//! **A second, audio-only Starfish pipeline**, independent of the player's. The player's engine
//! cannot carry it: its Load payloads declare a video codec, `ff::demux` treats a file with no
//! video stream as fatal, and the trailer preview already owns its single slot. So the C seam
//! keeps a separate theme slot (`sf_theme_*` in `src/starfish.c`): no video plane, no ACB, no
//! window, fed app-decoded PCM, at a volume set per pipeline.
//!
//! Phase 0 of the plan is a device spike (`spike`): before any of the feature is built on top of
//! it, prove on a television that the firmware accepts this second, audio-only object while the
//! trailer preview's pipeline is playing, and that it can be destroyed and loaded again.
use std::os::raw::{c_char, c_int, c_uint};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicI64, AtomicU32, Ordering};

// Only the spike calls the seam so far; the feature proper makes this unconditional.
#[cfg(all(feature = "devtriggers", not(feature = "hostsim"), not(test)))]
pub(crate) mod ffi;
#[cfg(all(feature = "devtriggers", not(feature = "hostsim"), not(test)))]
pub mod spike;

/// Callback events the theme object raised, and the last one's type and number: what the spike
/// reports. The theme pipeline's own UnloadCompleted (23) is latched by the C seam itself.
static EVENTS: AtomicU32 = AtomicU32::new(0);
static LAST_TYPE: AtomicI64 = AtomicI64::new(-1);
static LAST_NUM: AtomicI64 = AtomicI64::new(0);

/// `(events, last type, last num)` the theme object raised since boot.
pub fn event_counts() -> (u32, i64, i64) {
    (
        EVENTS.load(Ordering::Relaxed),
        LAST_TYPE.load(Ordering::Relaxed),
        LAST_NUM.load(Ordering::Relaxed),
    )
}

/// The theme object's Starfish callback (`src/starfish.c`'s `sf_theme_cb`), on the library's
/// thread. Panic-guarded (unwinding into C is UB); touches only this module's atomics.
#[no_mangle]
pub extern "C" fn sf_theme_on_event(epoch: c_uint, ty: c_int, num: i64, s: *const c_char) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        EVENTS.fetch_add(1, Ordering::Relaxed);
        LAST_TYPE.store(i64::from(ty), Ordering::Relaxed);
        LAST_NUM.store(num, Ordering::Relaxed);
        // Position ticks (type 0) are too frequent to log; every other event is rare.
        if ty != 0 {
            let text = if s.is_null() {
                String::new()
            } else {
                // SAFETY: the library passes a NUL-terminated string or null for the call.
                unsafe { std::ffi::CStr::from_ptr(s) }.to_string_lossy().into_owned()
            };
            plx_base::eventlog::log(&format!("theme: smp_cb epoch={epoch} type={ty} num={num} {text}"));
        }
    }));
}
