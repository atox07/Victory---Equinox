//! Built-in standalone upscaler stand-in for Series S (`--console series-s`).
//!
//! As Series S the title renders at 1280x720 and asks AMD's console upscaler
//! (`amd_fidelityfx_upscaler_xbox.dll`) to produce the 1920x1080 image. Its
//! console-compiled compute programs (`ACCC`, magic `FSPX`) are not translated,
//! so without a stand-in or an external PC DLL the output stays black.
//!
//! When no external `VICTORY_FSR_DLL` is configured, the title's lookup of
//! `ffxDispatch` is routed to [`dispatch`], which runs our single-pass sharp
//! upscaler (`crate::list::upscale`) directly on the host GPU with zero
//! external dependencies. `VICTORY_UPSCALE=title` leaves it unhooked.

use std::ffi::c_void;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering::Relaxed};

/// The upscaler's own `ffxDispatch`.
static ORIGINAL: AtomicUsize = AtomicUsize::new(0);

/// Dispatches seen; upscales drawn by us; upscales handed on.
static CALLS: [AtomicU64; 3] = [const { AtomicU64::new(0) }; 3];

/// `ffxDispatch(context, description)`.
type Dispatch = extern "system" fn(*mut c_void, *const u8) -> u32;

/// The description type of an upscale dispatch (`ffx_upscale.h`).
const UPSCALE: u64 = 0x0001_0001;

/// What the title's lookup of the upscaler's `ffxDispatch` gets: ours, which
/// keeps `original` for non-upscale dispatches.
pub fn hook(original: usize) -> usize {
    if std::env::var_os("VICTORY_UPSCALE").is_some_and(|v| v == "title") {
        return original;
    }
    ORIGINAL.store(original, Relaxed);
    tracing::info!(
        original = format_args!("{original:#x}"),
        "built-in sharp upscaler hooked for ffxDispatch (VICTORY_UPSCALE=title to disable)"
    );
    dispatch as *const () as usize
}

fn hand_on(context: *mut c_void, description: *const u8) -> u32 {
    match ORIGINAL.load(Relaxed) {
        0 => 1,
        // SAFETY: the address the upscaler exports its dispatch at.
        f => {
            let f = unsafe { std::mem::transmute::<usize, Dispatch>(f) };
            f(context, description)
        }
    }
}

extern "system" fn dispatch(context: *mut c_void, description: *const u8) -> u32 {
    if description.is_null() {
        return hand_on(context, description);
    }
    // SAFETY: the title passes a description, which begins with its type.
    let kind = unsafe { description.cast::<u64>().read_unaligned() };
    let call = CALLS[0].fetch_add(1, Relaxed);
    if kind != UPSCALE {
        if call < 8 {
            tracing::info!(
                kind = format_args!("{kind:#x}"),
                "built-in upscaler: non-upscale dispatch handed on"
            );
        }
        return hand_on(context, description);
    }
    // `ffxDispatchDescUpscale`: 16-byte header, command list, 7 resources of
    // 48 bytes (pointer first: 0=colour, 1=depth, 2=motionVectors, ..., 6=output),
    // 16 bytes jitter (f32x2) + motionVectorScale (f32x2), then render size and
    // upscale output size (u32x2 each).
    const LIST: usize = 16;
    const COLOUR: usize = 24;
    const MOTION: usize = 24 + 2 * 48;
    const OUTPUT: usize = 24 + 6 * 48;
    const JITTER: usize = OUTPUT + 48;
    const MV_SCALE: usize = JITTER + 8;
    const RENDER: usize = MV_SCALE + 8;
    const LEN: usize = RENDER + 16;
    // SAFETY: an upscale description is longer than `LEN`.
    let bytes = unsafe { std::slice::from_raw_parts(description, LEN) };
    let word = |at: usize| u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
    let flt = |at: usize| f32::from_bits(word(at));
    let pointer = |at: usize| (u64::from(word(at)) | u64::from(word(at + 4)) << 32) as usize;
    let (list, colour, motion, output) = (
        pointer(LIST),
        pointer(COLOUR),
        pointer(MOTION),
        pointer(OUTPUT),
    );
    let raw_jitter = [flt(JITTER), flt(JITTER + 4)];
    let raw_mv = [flt(MV_SCALE), flt(MV_SCALE + 4)];
    let jitter = [
        if raw_jitter[0].is_finite() && raw_jitter[0].abs() <= 2.0 { raw_jitter[0] } else { 0.0 },
        if raw_jitter[1].is_finite() && raw_jitter[1].abs() <= 2.0 { raw_jitter[1] } else { 0.0 },
    ];
    let mv_scale = [
        if raw_mv[0].is_finite() && raw_mv[0].abs() <= 16384.0 { raw_mv[0] } else { 0.0 },
        if raw_mv[1].is_finite() && raw_mv[1].abs() <= 16384.0 { raw_mv[1] } else { 0.0 },
    ];
    let render = [word(RENDER), word(RENDER + 4)];
    let size = [word(RENDER + 8), word(RENDER + 12)];
    if call < 2 {
        tracing::info!(
            list = format_args!("{list:#x}"),
            colour = format_args!("{colour:#x}"),
            motion = format_args!("{motion:#x}"),
            output = format_args!("{output:#x}"),
            ?jitter,
            ?mv_scale,
            ?render,
            ?size,
            "Neptune Clear-TAA: upscale dispatch"
        );
    }
    let sane = |d: [u32; 2]| (1..=16384).contains(&d[0]) && (1..=16384).contains(&d[1]);
    let drawn = list != 0
        && colour != 0
        && output != 0
        && sane(render)
        && sane(size)
        && crate::list::upscale(
            list as crate::object::This,
            colour as crate::object::This,
            motion as crate::object::This,
            output as crate::object::This,
            jitter,
            mv_scale,
            render,
            size,
        );
    if drawn {
        CALLS[1].fetch_add(1, Relaxed);
        return 0;
    }
    if CALLS[2].fetch_add(1, Relaxed) < 4 {
        tracing::warn!(?render, ?size, "built-in upscaler: fallback to title upscaler");
    }
    hand_on(context, description)
}
