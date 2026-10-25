//! theme::spike — Phase 0's device probe for the second, audio-only pipeline. Debug builds only
//! (`devtriggers`); it never runs in a shipped build.
//!
//! [`poll`] is called once per frame. It runs two experiments, each once per process, on a worker
//! that owns every `sf_theme_*` call:
//!
//! 1. **Alone**, 15 s after boot: two back-to-back cycles, proving the object loads, plays, honours
//!    `setVolume`, and is destroyed and loaded again.
//! 2. **Beside a trailer**: the first time the detail page's trailer preview is playing, one cycle
//!    while the video pipeline runs.
//!
//! A cycle is a 440 Hz tone of 12 s, fed as S16LE stereo 48 kHz PCM: 4 s at volume 30, 4 s at
//! 100, 4 s at 30. So a listener hears quiet-loud-quiet when `setVolume` works, and one level
//! throughout when it does not. Every step is logged as `theme spike: …` with the verb's result
//! and the process RSS.
use super::ffi;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

const RATE: u32 = 48_000;
const CHANNELS: u32 = 2;
/// One feed: 20 ms of audio.
const CHUNK_FRAMES: u32 = RATE / 50;
const TONE_SECS: u32 = 12;
/// How far ahead of the wall clock the feeder may run.
const FEED_AHEAD: Duration = Duration::from_millis(400);

const ALONE_AFTER: Duration = Duration::from_secs(15);

static BOOT: OnceLock<Instant> = OnceLock::new();
static RUNNING: AtomicBool = AtomicBool::new(false);
/// 0 nothing run, 1 the alone pair done, 2 the beside-a-trailer cycle done too.
static STAGE: AtomicU8 = AtomicU8::new(0);

/// Called every frame with whether the trailer preview is playing. Cheap: two atomic loads.
pub fn poll(preview_playing: bool) {
    let boot = *BOOT.get_or_init(Instant::now);
    if RUNNING.load(Ordering::Acquire) {
        return;
    }
    let stage = STAGE.load(Ordering::Acquire);
    let next = match stage {
        0 if boot.elapsed() >= ALONE_AFTER => Some(("alone", 2)),
        1 if preview_playing => Some(("beside a trailer", 1)),
        _ => None,
    };
    let Some((what, cycles)) = next else { return };
    RUNNING.store(true, Ordering::Release);
    let started = plx_base::task::spawn("theme-spike", move || {
        for n in 1..=cycles {
            log(&format!("{what}: cycle {n}/{cycles} begins"));
            cycle(n);
        }
        STAGE.store(stage + 1, Ordering::Release);
        RUNNING.store(false, Ordering::Release);
    });
    if started.is_none() {
        log("could not start the worker");
        STAGE.store(2, Ordering::Release);
        RUNNING.store(false, Ordering::Release);
    }
}

fn log(line: &str) {
    plx_base::eventlog::log(&format!("theme spike: {line} rss_kb={}", rss_kb()));
}

fn rss_kb() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find_map(|l| l.strip_prefix("VmRSS:"))
                .and_then(|v| v.trim().trim_end_matches("kB").trim().parse().ok())
        })
        .unwrap_or(0)
}

fn payload() -> String {
    format!(
        concat!(
            r#"{{"args":[{{"mediaTransportType":"BUFFERSTREAM","option":{{"appId":"{app}","#,
            r#""externalStreamingInfo":{{"contents":{{"codec":{{"audio":"PCM"}},"#,
            r#""esInfo":{{"pauseAtDecodeTime":false,"ptsToDecode":0,"seperatedPTS":true}},"#,
            r#""format":"RAW","provider":"plxnative","#,
            r#""pcmInfo":{{"format":"S16LE","channelMode":"stereo","layout":"interleaved","sampleRate":{rate},"bitsPerSample":16}}}},"#,
            r#""restartStreaming":false,"bufferingCtrInfo":{{"bufferMaxLevel":0,"bufferMinLevel":0,"preBufferByte":0,"#,
            r#""qBufferLevelAudio":0,"qBufferLevelVideo":0,"srcBufferLevelAudio":{{"minimum":1,"maximum":1048576}},"#,
            r#""srcBufferLevelVideo":{{"minimum":1,"maximum":1}}}}}},"#,
            r#""needAudio":true,"queryPosition":false,"lowDelayMode":false,"#,
            r#""transmission":{{"contentsType":"LIVE"}},"adaptiveStreaming":{{"audioOnly":true}}}}}}]}}"#
        ),
        app = plx_base::paths::app_id(),
        rate = RATE,
    )
}

/// One chunk of the tone starting at frame `first`, interleaved S16LE stereo at half scale.
fn tone(first: u64) -> Vec<u8> {
    let mut out = Vec::with_capacity((CHUNK_FRAMES * CHANNELS * 2) as usize);
    for i in 0..u64::from(CHUNK_FRAMES) {
        let t = (first + i) as f64 / f64::from(RATE);
        let s = ((t * 440.0 * std::f64::consts::TAU).sin() * 16_000.0) as i16;
        for _ in 0..CHANNELS {
            out.extend_from_slice(&s.to_le_bytes());
        }
    }
    out
}

fn cycle(n: u32) {
    // Distinct from the player's epochs only by convention; the theme callback is its own entry.
    let epoch = 0x7000_0000 + n + u32::from(STAGE.load(Ordering::Acquire)) * 16;
    let payload = std::ffi::CString::new(payload()).expect("payload has no NUL");
    let t0 = Instant::now();
    // SAFETY: payload is a valid NUL-terminated string for the call; this worker owns the theme
    // object for the whole cycle.
    let loaded = unsafe { ffi::sf_theme_load(payload.as_ptr(), epoch) };
    log(&format!("Load -> {loaded} in {} ms", t0.elapsed().as_millis()));
    if loaded == 0 {
        // SAFETY: as above.
        let destroyed = unsafe { ffi::sf_theme_release() };
        log(&format!("release after a refused Load -> {destroyed}"));
        return;
    }
    // SAFETY: as above, after Load returned on this thread.
    unsafe {
        log(&format!("setVolume(30) -> {}", ffi::sf_theme_set_volume(30)));
        log(&format!("Play -> {}", ffi::sf_theme_play()));
    }

    let total = u64::from(RATE * TONE_SECS);
    let mut frame = 0u64;
    let mut stage_volume = 30;
    let (mut ok, mut full, mut err) = (0u32, 0u32, 0u32);
    let start = Instant::now();
    while frame < total {
        let at = Duration::from_nanos(frame * 1_000_000_000 / u64::from(RATE));
        if at > start.elapsed() + FEED_AHEAD {
            std::thread::sleep(Duration::from_millis(5));
            continue;
        }
        let want = if start.elapsed() < Duration::from_secs(4) || start.elapsed() >= Duration::from_secs(8) {
            30
        } else {
            100
        };
        if want != stage_volume {
            // SAFETY: as above.
            let r = unsafe { ffi::sf_theme_set_volume(want) };
            log(&format!("setVolume({want}) -> {r} at {} ms", start.elapsed().as_millis()));
            stage_volume = want;
        }
        let chunk = tone(frame);
        let pts = (at.as_nanos()) as i64;
        // SAFETY: chunk is readable for its length for the call.
        let r = unsafe { ffi::sf_theme_feed(chunk.as_ptr(), chunk.len() as u32, pts) } as u8;
        match r {
            b'O' => {
                ok += 1;
                frame += u64::from(CHUNK_FRAMES);
            }
            b'B' => {
                full += 1;
                std::thread::sleep(Duration::from_millis(10));
            }
            _ => {
                err += 1;
                if err > 50 {
                    log("Feed keeps failing; giving up on this cycle");
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        if start.elapsed() > Duration::from_secs(u64::from(TONE_SECS) + 10) {
            log("feeding is not keeping up with the clock; giving up on this cycle");
            break;
        }
    }
    let (events, last_ty, last_num) = super::event_counts();
    log(&format!("fed ok={ok} full={full} err={err} events={events} last=({last_ty},{last_num})"));
    // SAFETY: as above.
    unsafe {
        log(&format!("pushEOS -> {}", ffi::sf_theme_push_eos()));
    }
    std::thread::sleep(FEED_AHEAD + Duration::from_millis(500));
    // SAFETY: as above.
    unsafe { ffi::sf_theme_unload() };
    let waited = Instant::now();
    // SAFETY: as above.
    while unsafe { ffi::sf_theme_unload_completed() } == 0 && waited.elapsed() < Duration::from_secs(3) {
        std::thread::sleep(Duration::from_millis(20));
    }
    // SAFETY: as above.
    let completed = unsafe { ffi::sf_theme_unload_completed() };
    log(&format!("Unload completed={completed} after {} ms", waited.elapsed().as_millis()));
    // SAFETY: as above.
    let destroyed = unsafe { ffi::sf_theme_release() };
    log(&format!("release -> destroyed={destroyed}"));
}
