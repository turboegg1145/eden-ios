//! C ABI for the pure-Rust Musica runtime.
//!
//! The iOS host links `libeden_ffi.a` and includes `include/eden_ffi.h`.
//! All pointers returned by this module are borrowed and remain valid until
//! the next call that mutates the same object (documented per function).

pub mod runtime;

use std::ffi::{c_char, CStr, CString};
use std::path::Path;
use std::ptr;
use std::sync::{Mutex, MutexGuard};

pub use runtime::{Assets, Runtime, RuntimeError};

/// Matches `EdenFrameInfo` in `eden_ffi.h`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct EdenFrameInfo {
    pub width: u32,
    pub height: u32,
    pub stride: u32,
}

struct Inner {
    runtime: Option<Runtime>,
    state_json: CString,
    events_json: CString,
    error: CString,
}

/// Opaque handle exposed to C as `EdenEngine *`.
pub struct EdenEngine {
    inner: Mutex<Inner>,
}

fn lock(engine: *mut EdenEngine) -> Option<MutexGuard<'static, Inner>> {
    if engine.is_null() {
        return None;
    }
    // SAFETY: the caller owns a pointer returned by `eden_engine_create` and
    // must not use it after `eden_engine_destroy`.
    let engine = unsafe { &*engine };
    Some(match engine.inner.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    })
}

fn cstr(ptr: *const c_char) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    // SAFETY: the C caller guarantees a NUL-terminated string.
    unsafe { CStr::from_ptr(ptr) }
        .to_str()
        .ok()
        .map(ToOwned::to_owned)
}

fn set_error(inner: &mut Inner, message: impl Into<String>) {
    let message = message.into().replace('\0', " ");
    inner.error = CString::new(message).unwrap_or_default();
}

fn clear_error(inner: &mut Inner) {
    inner.error = CString::new("").unwrap();
}

fn cstring(value: String) -> CString {
    CString::new(value.replace('\0', " ")).unwrap_or_default()
}

fn no_runtime(inner: &mut Inner) -> i32 {
    set_error(inner, "engine is not open; call eden_engine_open first");
    -1
}

/// Creates a runtime handle. Returns NULL only on allocation failure.
#[no_mangle]
pub extern "C" fn eden_engine_create() -> *mut EdenEngine {
    Box::into_raw(Box::new(EdenEngine {
        inner: Mutex::new(Inner {
            runtime: None,
            state_json: CString::new("{}").unwrap(),
            events_json: CString::new("[]").unwrap(),
            error: CString::new("").unwrap(),
        }),
    }))
}

/// Destroys a handle created by [`eden_engine_create`].
#[no_mangle]
pub unsafe extern "C" fn eden_engine_destroy(engine: *mut EdenEngine) {
    if !engine.is_null() {
        drop(Box::from_raw(engine));
    }
}

/// Opens `game_dir` (`scr.paz`, `bg.paz`, `sys.paz`, optional archives).
///
/// `exe_path` may be NULL; it is only needed for archives whose id is not in
/// the embedded key tables. Returns 0 on success, -1 on error.
#[no_mangle]
pub unsafe extern "C" fn eden_engine_open(
    engine: *mut EdenEngine,
    game_dir: *const c_char,
    exe_path: *const c_char,
) -> i32 {
    let Some(mut inner) = lock(engine) else {
        return -1;
    };
    let Some(dir) = cstr(game_dir) else {
        set_error(&mut inner, "game_dir is NULL or not UTF-8");
        return -1;
    };
    let exe = cstr(exe_path).map(|p| Path::new(&p).to_path_buf());
    match Runtime::open(&dir, exe.as_deref()) {
        Ok(runtime) => {
            inner.runtime = Some(runtime);
            clear_error(&mut inner);
            0
        }
        Err(e) => {
            set_error(&mut inner, e.to_string());
            -1
        }
    }
}

/// Opens `game_dir` and redirects config/unlocks/save slots to `save_dir`.
///
/// Use this on iOS when the archives are inside the read-only app bundle and
/// saves must go to `Documents/saves`. `save_dir` may be NULL, which keeps the
/// temporary-directory default of [`eden_engine_open`]. Returns 0 on success.
#[no_mangle]
pub unsafe extern "C" fn eden_engine_open_with_save_dir(
    engine: *mut EdenEngine,
    game_dir: *const c_char,
    exe_path: *const c_char,
    save_dir: *const c_char,
) -> i32 {
    let Some(mut inner) = lock(engine) else {
        return -1;
    };
    let Some(dir) = cstr(game_dir) else {
        set_error(&mut inner, "game_dir is NULL or not UTF-8");
        return -1;
    };
    let exe = cstr(exe_path).map(|p| Path::new(&p).to_path_buf());
    let saves = cstr(save_dir).map(|p| Path::new(&p).to_path_buf());
    match Runtime::open_with_save_dir(&dir, exe.as_deref(), saves.as_deref()) {
        Ok(runtime) => {
            inner.runtime = Some(runtime);
            clear_error(&mut inner);
            0
        }
        Err(e) => {
            set_error(&mut inner, e.to_string());
            -1
        }
    }
}

/// Starts a script by file name (`A001.sc`). Returns 0 on success.
#[no_mangle]
pub unsafe extern "C" fn eden_engine_start_script(
    engine: *mut EdenEngine,
    script_name: *const c_char,
) -> i32 {
    let Some(mut inner) = lock(engine) else {
        return -1;
    };
    let Some(name) = cstr(script_name) else {
        set_error(&mut inner, "script_name is NULL or not UTF-8");
        return -1;
    };
    let result = match inner.runtime.as_mut() {
        Some(runtime) => runtime.start_script(&name),
        None => return no_runtime(&mut inner),
    };
    match result {
        Ok(()) => {
            clear_error(&mut inner);
            0
        }
        Err(e) => {
            set_error(&mut inner, e.to_string());
            -1
        }
    }
}

/// Advances exactly one 60 Hz frame. Returns 0 on success.
#[no_mangle]
pub unsafe extern "C" fn eden_engine_tick(engine: *mut EdenEngine) -> i32 {
    let Some(mut inner) = lock(engine) else {
        return -1;
    };
    let result = match inner.runtime.as_mut() {
        Some(runtime) => runtime.tick(),
        None => return no_runtime(&mut inner),
    };
    match result {
        Ok(()) => 0,
        Err(e) => {
            set_error(&mut inner, e.to_string());
            -1
        }
    }
}

/// Advances by wall-clock seconds (for example 1/120 on a ProMotion display)
/// and interpolates scrolling/transitions between 60 Hz logical frames.
/// Returns the number of logical frames consumed, or -1 on error.
#[no_mangle]
pub unsafe extern "C" fn eden_engine_tick_display(
    engine: *mut EdenEngine,
    dt_seconds: f64,
) -> i32 {
    let Some(mut inner) = lock(engine) else {
        return -1;
    };
    let result = match inner.runtime.as_mut() {
        Some(runtime) => runtime.tick_display(dt_seconds),
        None => return no_runtime(&mut inner),
    };
    match result {
        Ok(ticks) => ticks as i32,
        Err(e) => {
            set_error(&mut inner, e.to_string());
            -1
        }
    }
}

/// Drops the wall-clock remainder accumulated by `eden_engine_tick_display`.
/// Call after returning from the background so the first frame does not
/// fast-forward through a long pause.
#[no_mangle]
pub unsafe extern "C" fn eden_engine_reset_display_clock(engine: *mut EdenEngine) {
    if let Some(mut inner) = lock(engine) {
        if let Some(runtime) = inner.runtime.as_mut() {
            runtime.reset_display_clock();
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn eden_engine_click(engine: *mut EdenEngine) {
    if let Some(mut inner) = lock(engine) {
        if let Some(runtime) = inner.runtime.as_mut() {
            runtime.click();
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn eden_engine_set_auto_advance(engine: *mut EdenEngine, enabled: i32) {
    if let Some(mut inner) = lock(engine) {
        if let Some(runtime) = inner.runtime.as_mut() {
            runtime.set_auto_advance(enabled != 0);
        }
    }
}

/// Renders the current scene and returns a pointer to tightly packed RGBA8
/// pixels. The pointer is valid until the next render/tick/load call.
#[no_mangle]
pub unsafe extern "C" fn eden_engine_render(
    engine: *mut EdenEngine,
    info: *mut EdenFrameInfo,
) -> *const u8 {
    let Some(mut inner) = lock(engine) else {
        return ptr::null();
    };
    let Some(runtime) = inner.runtime.as_mut() else {
        set_error(
            &mut inner,
            "engine is not open; call eden_engine_open first",
        );
        return ptr::null();
    };
    runtime.render_now();
    let frame = runtime.frame();
    if !info.is_null() {
        *info = EdenFrameInfo {
            width: frame.width,
            height: frame.height,
            stride: frame.width * 4,
        };
    }
    frame.data.as_ptr()
}

/// Returns the last rendered RGBA8 frame without rendering. Valid until the
/// next mutating call.
#[no_mangle]
pub unsafe extern "C" fn eden_engine_pixels(engine: *mut EdenEngine) -> *const u8 {
    let Some(inner) = lock(engine) else {
        return ptr::null();
    };
    inner
        .runtime
        .as_ref()
        .map(|runtime| runtime.frame().data.as_ptr())
        .unwrap_or(ptr::null())
}

/// Writes the current framebuffer to `path` as PNG. Returns 0 on success.
#[no_mangle]
pub unsafe extern "C" fn eden_engine_render_png(
    engine: *mut EdenEngine,
    path: *const c_char,
) -> i32 {
    let Some(mut inner) = lock(engine) else {
        return -1;
    };
    let Some(path) = cstr(path) else {
        set_error(&mut inner, "path is NULL or not UTF-8");
        return -1;
    };
    let result = match inner.runtime.as_mut() {
        Some(runtime) => runtime.render_png(path),
        None => return no_runtime(&mut inner),
    };
    match result {
        Ok(()) => 0,
        Err(e) => {
            set_error(&mut inner, e.to_string());
            -1
        }
    }
}

/// Loads a TTF/OTF font for the message panel. Returns 0 on success.
#[no_mangle]
pub unsafe extern "C" fn eden_engine_load_font(
    engine: *mut EdenEngine,
    path: *const c_char,
) -> i32 {
    let Some(mut inner) = lock(engine) else {
        return -1;
    };
    let Some(path) = cstr(path) else {
        set_error(&mut inner, "font path is NULL or not UTF-8");
        return -1;
    };
    let result = match inner.runtime.as_mut() {
        Some(runtime) => runtime.load_font(path),
        None => return no_runtime(&mut inner),
    };
    match result {
        Ok(()) => 0,
        Err(e) => {
            set_error(&mut inner, e.to_string());
            -1
        }
    }
}

/// Serialises the VM save state to JSON. The pointer is valid until the next
/// call on this handle.
#[no_mangle]
pub unsafe extern "C" fn eden_engine_state_json(engine: *mut EdenEngine) -> *const c_char {
    let Some(mut inner) = lock(engine) else {
        return ptr::null();
    };
    let json = match inner.runtime.as_ref() {
        Some(runtime) => runtime.state_json(),
        None => {
            set_error(
                &mut inner,
                "engine is not open; call eden_engine_open first",
            );
            return ptr::null();
        }
    };
    match json {
        Ok(json) => {
            inner.state_json = cstring(json);
            clear_error(&mut inner);
            inner.state_json.as_ptr()
        }
        Err(e) => {
            set_error(&mut inner, e.to_string());
            ptr::null()
        }
    }
}

/// Serialises the events produced by the last tick. The pointer is valid
/// until the next call on this handle.
#[no_mangle]
pub unsafe extern "C" fn eden_engine_events_json(engine: *mut EdenEngine) -> *const c_char {
    let Some(mut inner) = lock(engine) else {
        return ptr::null();
    };
    let json = match inner.runtime.as_ref() {
        Some(runtime) => runtime.events_json(),
        None => {
            set_error(
                &mut inner,
                "engine is not open; call eden_engine_open first",
            );
            return ptr::null();
        }
    };
    match json {
        Ok(json) => {
            inner.events_json = cstring(json);
            clear_error(&mut inner);
            inner.events_json.as_ptr()
        }
        Err(e) => {
            set_error(&mut inner, e.to_string());
            ptr::null()
        }
    }
}

/// Saves the current state to `path` (JSON). Returns 0 on success.
#[no_mangle]
pub unsafe extern "C" fn eden_engine_save(engine: *mut EdenEngine, path: *const c_char) -> i32 {
    let Some(mut inner) = lock(engine) else {
        return -1;
    };
    let Some(path) = cstr(path) else {
        set_error(&mut inner, "path is NULL or not UTF-8");
        return -1;
    };
    let result = match inner.runtime.as_ref() {
        Some(runtime) => runtime.save_to(path),
        None => return no_runtime(&mut inner),
    };
    match result {
        Ok(()) => 0,
        Err(e) => {
            set_error(&mut inner, e.to_string());
            -1
        }
    }
}

/// Loads a state previously written by [`eden_engine_save`]. Returns 0 on
/// success.
#[no_mangle]
pub unsafe extern "C" fn eden_engine_load(engine: *mut EdenEngine, path: *const c_char) -> i32 {
    let Some(mut inner) = lock(engine) else {
        return -1;
    };
    let Some(path) = cstr(path) else {
        set_error(&mut inner, "path is NULL or not UTF-8");
        return -1;
    };
    let result = match inner.runtime.as_mut() {
        Some(runtime) => runtime.load_from(path),
        None => return no_runtime(&mut inner),
    };
    match result {
        Ok(()) => 0,
        Err(e) => {
            set_error(&mut inner, e.to_string());
            -1
        }
    }
}

/// Starts a voice clip from `voice.paz`. `name` is the extension-less entry
/// name (`sio-C013-0003`). Returns 0 on success.
#[no_mangle]
pub unsafe extern "C" fn eden_engine_play_voice(
    engine: *mut EdenEngine,
    name: *const c_char,
) -> i32 {
    let Some(mut inner) = lock(engine) else {
        return -1;
    };
    let Some(name) = cstr(name) else {
        set_error(&mut inner, "voice name is NULL or not UTF-8");
        return -1;
    };
    let result = match inner.runtime.as_mut() {
        Some(runtime) => runtime.play_voice(&name, None),
        None => return no_runtime(&mut inner),
    };
    match result {
        Ok(()) => 0,
        Err(e) => {
            set_error(&mut inner, e.to_string());
            -1
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn eden_engine_stop_voice(engine: *mut EdenEngine) {
    if let Some(mut inner) = lock(engine) {
        if let Some(runtime) = inner.runtime.as_mut() {
            runtime.stop_voice();
        }
    }
}

/// Mixes up to `frames` interleaved stereo float frames into `out`. This is
/// called from the CoreAudio render callback.
#[no_mangle]
pub unsafe extern "C" fn eden_engine_mix_audio(
    engine: *mut EdenEngine,
    out: *mut f32,
    frames: u32,
) -> u32 {
    if out.is_null() || frames == 0 {
        return 0;
    }
    let Some(mut inner) = lock(engine) else {
        return 0;
    };
    let Some(runtime) = inner.runtime.as_mut() else {
        return 0;
    };
    let out = std::slice::from_raw_parts_mut(out as *mut [f32; 2], frames as usize);
    runtime.mix_audio(out) as u32
}

#[no_mangle]
pub unsafe extern "C" fn eden_engine_lipsync_frame(engine: *mut EdenEngine) -> u32 {
    lock(engine)
        .and_then(|inner| {
            inner
                .runtime
                .as_ref()
                .map(|runtime| runtime.lipsync_frame())
        })
        .unwrap_or(0)
}

#[no_mangle]
pub unsafe extern "C" fn eden_engine_lipsync_rms(engine: *mut EdenEngine) -> f32 {
    lock(engine)
        .and_then(|inner| inner.runtime.as_ref().map(|runtime| runtime.lipsync_rms()))
        .unwrap_or(0.0)
}

#[no_mangle]
pub unsafe extern "C" fn eden_engine_frame_count(engine: *mut EdenEngine) -> u64 {
    lock(engine)
        .and_then(|inner| inner.runtime.as_ref().map(|runtime| runtime.frame_count()))
        .unwrap_or(0)
}

/// Returns the last error message (empty when the last call succeeded). The
/// pointer is valid until the next call on this handle.
#[no_mangle]
pub unsafe extern "C" fn eden_engine_last_error(engine: *mut EdenEngine) -> *const c_char {
    let Some(inner) = lock(engine) else {
        return ptr::null();
    };
    inner.error.as_ptr()
}

/// ABI version, bumped when function signatures change.
#[no_mangle]
pub extern "C" fn eden_ffi_version() -> u32 {
    3
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game_dir() -> Option<std::path::PathBuf> {
        let path = std::path::PathBuf::from("/mnt/f/gal/【PC硬盘】【官中】eden");
        path.exists().then_some(path)
    }

    #[test]
    fn header_declares_every_exported_function() {
        let source = include_str!("lib.rs");
        let header = include_str!("../include/eden_ffi.h");
        for line in source.lines() {
            let Some(rest) = line.strip_prefix("pub unsafe extern \"C\" fn ") else {
                continue;
            };
            let Some(name) = rest.split('(').next() else {
                continue;
            };
            assert!(
                header.contains(name),
                "eden_ffi.h is missing declaration for {name}"
            );
        }
        assert!(header.contains("eden_ffi_version"));
    }

    #[test]
    fn ffi_smoke_uses_real_archives() {
        let Some(dir) = game_dir() else {
            eprintln!("skip: game dir missing");
            return;
        };
        unsafe {
            let engine = eden_engine_create();
            assert!(!engine.is_null());
            let dir = CString::new(dir.to_str().unwrap()).unwrap();
            assert_eq!(eden_engine_open(engine, dir.as_ptr(), ptr::null()), 0);
            let name = CString::new("A001.sc").unwrap();
            assert_eq!(eden_engine_start_script(engine, name.as_ptr()), 0);
            eden_engine_set_auto_advance(engine, 1);
            for _ in 0..120 {
                assert_eq!(eden_engine_tick(engine), 0);
            }
            let mut info = EdenFrameInfo::default();
            let pixels = eden_engine_render(engine, &mut info);
            assert!(!pixels.is_null());
            assert_eq!((info.width, info.height, info.stride), (1024, 640, 4096));
            assert!(eden_engine_frame_count(engine) >= 120);
            assert!(!eden_engine_state_json(engine).is_null());
            assert!(!eden_engine_events_json(engine).is_null());
            eden_engine_destroy(engine);
        }
    }
}

// ---------------------------------------------------------------------------
// Full UI, save-slot and video controls (ABI 2)
// ---------------------------------------------------------------------------

fn ui_screen_from_int(value: i32) -> Option<eden_render::ui::UiScreen> {
    use eden_render::ui::UiScreen;
    Some(match value {
        0 => UiScreen::Title,
        1 => UiScreen::Save,
        2 => UiScreen::Load,
        3 => UiScreen::Backlog,
        4 => UiScreen::Gallery,
        5 => UiScreen::Music,
        6 => UiScreen::Config,
        7 => UiScreen::Extra,
        8 => UiScreen::Game,
        _ => return None,
    })
}

fn ui_screen_to_int(screen: eden_render::ui::UiScreen) -> i32 {
    use eden_render::ui::UiScreen;
    match screen {
        UiScreen::Title => 0,
        UiScreen::Save => 1,
        UiScreen::Load => 2,
        UiScreen::Backlog => 3,
        UiScreen::Gallery => 4,
        UiScreen::GalleryView => 9,
        UiScreen::Music => 5,
        UiScreen::Config => 6,
        UiScreen::Extra => 7,
        UiScreen::Game => 8,
    }
}

/// Opens one of the full-screen UI modes (0=title, 1=save, 2=load,
/// 3=backlog, 4=gallery, 5=music, 6=config, 7=extra, 8=game).
#[no_mangle]
pub unsafe extern "C" fn eden_engine_ui_open(engine: *mut EdenEngine, screen: i32) -> i32 {
    let Some(mut inner) = lock(engine) else {
        return -1;
    };
    let Some(screen) = ui_screen_from_int(screen) else {
        set_error(&mut inner, format!("unknown UI screen {screen}"));
        return -1;
    };
    let Some(runtime) = inner.runtime.as_mut() else {
        return no_runtime(&mut inner);
    };
    runtime.open_ui(screen);
    clear_error(&mut inner);
    0
}

/// Current UI screen (see `eden_engine_ui_open`).
#[no_mangle]
pub unsafe extern "C" fn eden_engine_ui_screen(engine: *mut EdenEngine) -> i32 {
    lock(engine)
        .and_then(|inner| inner.runtime.as_ref().map(|r| ui_screen_to_int(r.ui().screen)))
        .unwrap_or(-1)
}

/// Forwards a primary tap in 1024×640 stage coordinates.
#[no_mangle]
pub unsafe extern "C" fn eden_engine_ui_tap(engine: *mut EdenEngine, x: i32, y: i32) -> i32 {
    let Some(mut inner) = lock(engine) else {
        return -1;
    };
    let Some(runtime) = inner.runtime.as_mut() else {
        return no_runtime(&mut inner);
    };
    if runtime.ui_tap(x, y) {
        0
    } else {
        1
    }
}

#[no_mangle]
pub unsafe extern "C" fn eden_engine_ui_move(engine: *mut EdenEngine, x: i32, y: i32) {
    if let Some(mut inner) = lock(engine) {
        if let Some(runtime) = inner.runtime.as_mut() {
            runtime.ui_move(x, y);
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn eden_engine_ui_scroll(engine: *mut EdenEngine, delta: i32) {
    if let Some(mut inner) = lock(engine) {
        if let Some(runtime) = inner.runtime.as_mut() {
            runtime.ui_scroll(delta);
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn eden_engine_ui_swipe_up(engine: *mut EdenEngine) {
    if let Some(mut inner) = lock(engine) {
        if let Some(runtime) = inner.runtime.as_mut() {
            runtime.ui_swipe_up();
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn eden_engine_ui_two_finger_tap(engine: *mut EdenEngine) {
    if let Some(mut inner) = lock(engine) {
        if let Some(runtime) = inner.runtime.as_mut() {
            runtime.ui_two_finger_tap();
        }
    }
}

/// Serialises the ten save slots (without thumbnails) as JSON.
#[no_mangle]
pub unsafe extern "C" fn eden_engine_ui_slots_json(engine: *mut EdenEngine) -> *const c_char {
    let Some(mut inner) = lock(engine) else {
        return ptr::null();
    };
    let Some(runtime) = inner.runtime.as_ref() else {
        no_runtime(&mut inner);
        return ptr::null();
    };
    let slots: Vec<serde_json::Value> = runtime
        .ui()
        .slots
        .iter()
        .map(|slot| {
            serde_json::json!({
                "index": slot.index,
                "empty": slot.empty,
                "timestamp": slot.timestamp,
                "summary": slot.summary,
                "script": slot.script,
            })
        })
        .collect();
    inner.state_json = cstring(serde_json::to_string(&slots).unwrap_or_else(|_| "[]".into()));
    inner.state_json.as_ptr()
}

#[no_mangle]
pub unsafe extern "C" fn eden_engine_ui_save_slot(engine: *mut EdenEngine, index: u32) -> i32 {
    let Some(mut inner) = lock(engine) else {
        return -1;
    };
    let result = match inner.runtime.as_mut() {
        Some(runtime) => runtime.save_slot(index as usize),
        None => return no_runtime(&mut inner),
    };
    match result {
        Ok(()) => {
            clear_error(&mut inner);
            0
        }
        Err(e) => {
            set_error(&mut inner, e.to_string());
            -1
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn eden_engine_ui_load_slot(engine: *mut EdenEngine, index: u32) -> i32 {
    let Some(mut inner) = lock(engine) else {
        return -1;
    };
    let result = match inner.runtime.as_mut() {
        Some(runtime) => runtime.load_slot(index as usize),
        None => return no_runtime(&mut inner),
    };
    match result {
        Ok(()) => {
            clear_error(&mut inner);
            0
        }
        Err(e) => {
            set_error(&mut inner, e.to_string());
            -1
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn eden_engine_set_save_dir(
    engine: *mut EdenEngine,
    path: *const c_char,
) -> i32 {
    let Some(mut inner) = lock(engine) else {
        return -1;
    };
    let Some(path) = cstr(path) else {
        set_error(&mut inner, "save directory is NULL or not UTF-8");
        return -1;
    };
    let result = match inner.runtime.as_mut() {
        Some(runtime) => runtime.set_save_dir(path),
        None => return no_runtime(&mut inner),
    };
    match result {
        Ok(()) => {
            clear_error(&mut inner);
            0
        }
        Err(e) => {
            set_error(&mut inner, e.to_string());
            -1
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn eden_engine_set_video_enabled(engine: *mut EdenEngine, enabled: i32) {
    if let Some(mut inner) = lock(engine) {
        if let Some(runtime) = inner.runtime.as_mut() {
            runtime.set_video_enabled(enabled != 0);
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn eden_engine_movie_active(engine: *mut EdenEngine) -> i32 {
    lock(engine)
        .and_then(|inner| inner.runtime.as_ref().map(|r| r.movie_active() as i32))
        .unwrap_or(0)
}

/// Switches the active edition: 0 = main, 1 = PLUS+MOSAIC.
#[no_mangle]
pub unsafe extern "C" fn eden_engine_open_edition(engine: *mut EdenEngine, edition: i32) -> i32 {
    let Some(mut inner) = lock(engine) else {
        return -1;
    };
    let edition = match edition {
        0 => eden_paz::Edition::Main,
        1 => eden_paz::Edition::PlusMosaic,
        other => {
            set_error(&mut inner, format!("unknown edition {other}"));
            return -1;
        }
    };
    let result = match inner.runtime.as_mut() {
        Some(runtime) => runtime.open_edition(edition),
        None => return no_runtime(&mut inner),
    };
    match result {
        Ok(()) => {
            clear_error(&mut inner);
            0
        }
        Err(e) => {
            set_error(&mut inner, e.to_string());
            -1
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn eden_engine_save_unlocks(engine: *mut EdenEngine) {
    if let Some(inner) = lock(engine) {
        if let Some(runtime) = inner.runtime.as_ref() {
            runtime.save_unlocks();
        }
    }
}

/// 1 when the user asked to quit from the title menu.
#[no_mangle]
pub unsafe extern "C" fn eden_engine_quit_requested(engine: *mut EdenEngine) -> i32 {
    lock(engine)
        .and_then(|inner| inner.runtime.as_ref().map(|r| r.quit_requested() as i32))
        .unwrap_or(0)
}
