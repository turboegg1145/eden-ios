/*
 * eden_ffi.h — C ABI for the pure-Rust Musica runtime (eden-rs).
 *
 * Target: iOS 12.0 – 14.4 (arm64), Metal 2.0 / OpenGL ES 3.0 hosts.
 *
 * Ownership and lifetime
 * ----------------------
 *   EdenEngine * is created by eden_engine_create() and must be destroyed
 *   with eden_engine_destroy(). All other pointers are borrowed:
 *
 *   - eden_engine_render()/eden_engine_pixels() return RGBA8 pixels owned by
 *     the engine. They stay valid until the next call that mutates the
 *     engine (tick/render/load/open) on the same thread.
 *   - eden_engine_state_json(), eden_engine_events_json() and
 *     eden_engine_last_error() return NUL-terminated UTF-8 owned by the
 *     engine. Copy the string before the next call on the same handle.
 *
 * Threading
 * ---------
 *   The engine is internally locked. It is safe to call
 *   eden_engine_mix_audio() from the CoreAudio render callback while the
 *   display link calls eden_engine_tick()/eden_engine_render().
 *   Do not call eden_engine_destroy() while another thread is inside a call.
 */

#ifndef EDEN_FFI_H
#define EDEN_FFI_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#if defined(__GNUC__) || defined(__clang__)
#define EDEN_FFI_API __attribute__((visibility("default")))
#else
#define EDEN_FFI_API
#endif

#define EDEN_STAGE_WIDTH 1024u
#define EDEN_STAGE_HEIGHT 640u
#define EDEN_FFI_ABI_VERSION 3u

typedef struct EdenEngine EdenEngine;

typedef struct EdenFrameInfo {
    uint32_t width;   /* always 1024 */
    uint32_t height;  /* always 640 */
    uint32_t stride;  /* bytes per row = width * 4 */
} EdenFrameInfo;

/* Returns the ABI version (EDEN_FFI_ABI_VERSION). */
EDEN_FFI_API uint32_t eden_ffi_version(void);

/* Creates an engine handle. Returns NULL on allocation failure. */
EDEN_FFI_API EdenEngine *eden_engine_create(void);

/* Destroys an engine handle. Passing NULL is a no-op. */
EDEN_FFI_API void eden_engine_destroy(EdenEngine *engine);

/*
 * Opens a retail game directory containing scr.paz, bg.paz and sys.paz.
 * Optional archives (st/bgm/se/voice/mov) are opened when present.
 *
 * exe_path may be NULL. It is only required for archives whose 32-byte id is
 * not present in the embedded key tables (eden.exe / eden_pm_en.exe).
 *
 * Returns 0 on success, -1 on error (see eden_engine_last_error()).
 */
EDEN_FFI_API int eden_engine_open(EdenEngine *engine,
                                  const char *game_dir,
                                  const char *exe_path);

/*
 * Same as eden_engine_open(), but redirects config.json, unlocks.json and the
 * save slots to save_dir (for example Documents/saves on iOS, where game_dir
 * may be the read-only app bundle). save_dir may be NULL to keep the default.
 *
 * Returns 0 on success, -1 on error.
 */
EDEN_FFI_API int eden_engine_open_with_save_dir(EdenEngine *engine,
                                                const char *game_dir,
                                                const char *exe_path,
                                                const char *save_dir);

/* Starts a script such as "A001.sc". Returns 0 on success. */
EDEN_FFI_API int eden_engine_start_script(EdenEngine *engine,
                                          const char *script_name);

/* Advances exactly one 60 Hz logical frame. Returns 0 on success. */
EDEN_FFI_API int eden_engine_tick(EdenEngine *engine);

/*
 * Advances by wall-clock seconds (e.g. 1.0/120.0 from a ProMotion display
 * link) and interpolates scroll/transition state between 60 Hz logical
 * frames. Returns the number of logical frames consumed, or -1 on error.
 * Call eden_engine_render() afterwards to draw the interpolated frame.
 */
EDEN_FFI_API int eden_engine_tick_display(EdenEngine *engine, double dt_seconds);
EDEN_FFI_API void eden_engine_reset_display_clock(EdenEngine *engine);

/* Queues one click/touch advance for the next tick. */
EDEN_FFI_API void eden_engine_click(EdenEngine *engine);

/* Enables automatic advancement through `\a` wait-click markers. */
EDEN_FFI_API void eden_engine_set_auto_advance(EdenEngine *engine, int enabled);

/*
 * Renders the current scene into the internal RGBA8 framebuffer and returns
 * a pointer to it. `info` may be NULL. The pointer is valid until the next
 * mutating call.
 */
EDEN_FFI_API const uint8_t *eden_engine_render(EdenEngine *engine,
                                               EdenFrameInfo *info);

/* Returns the last rendered RGBA8 framebuffer without re-rendering. */
EDEN_FFI_API const uint8_t *eden_engine_pixels(EdenEngine *engine);

/* Writes the current framebuffer to a PNG file. Returns 0 on success. */
EDEN_FFI_API int eden_engine_render_png(EdenEngine *engine, const char *path);

/* Loads a TTF/OTF font used by the message panel. Returns 0 on success. */
EDEN_FFI_API int eden_engine_load_font(EdenEngine *engine, const char *path);

/* JSON save state (versioned, Serde-compatible). Borrowed string. */
EDEN_FFI_API const char *eden_engine_state_json(EdenEngine *engine);

/* JSON array of events produced by the last tick. Borrowed string. */
EDEN_FFI_API const char *eden_engine_events_json(EdenEngine *engine);

/* Writes/reads a JSON save file. Returns 0 on success. */
EDEN_FFI_API int eden_engine_save(EdenEngine *engine, const char *path);
EDEN_FFI_API int eden_engine_load(EdenEngine *engine, const char *path);

/*
 * Starts an extension-less voice entry from voice.paz (for example
 * "sio-C013-0003"). Returns 0 on success.
 */
EDEN_FFI_API int eden_engine_play_voice(EdenEngine *engine, const char *name);
EDEN_FFI_API void eden_engine_stop_voice(EdenEngine *engine);

/*
 * Mixes BGM/SE/voice into `frames` interleaved stereo float frames.
 * `out` must point to at least frames * 2 floats. Returns the number of
 * frames written (0 when nothing is playing). Safe to call from the
 * CoreAudio render callback.
 */
EDEN_FFI_API uint32_t eden_engine_mix_audio(EdenEngine *engine,
                                            float *out,
                                            uint32_t frames);

/* Current lip-sync mouth frame (0..frame_count-1) and RMS envelope. */
EDEN_FFI_API uint32_t eden_engine_lipsync_frame(EdenEngine *engine);
EDEN_FFI_API float eden_engine_lipsync_rms(EdenEngine *engine);

/* Number of logical 60 Hz frames advanced since start_script(). */
EDEN_FFI_API uint64_t eden_engine_frame_count(EdenEngine *engine);

/* Last error message; empty string when the last call succeeded. */
EDEN_FFI_API const char *eden_engine_last_error(EdenEngine *engine);

/* ------------------------------------------------------------------ */
/* Full UI, save slots and video (ABI 2)                              */
/* ------------------------------------------------------------------ */

/* 0=title 1=save 2=load 3=backlog 4=gallery 5=music 6=config 7=extra 8=game */
EDEN_FFI_API int eden_engine_ui_open(EdenEngine *engine, int screen);
EDEN_FFI_API int eden_engine_ui_screen(EdenEngine *engine);
EDEN_FFI_API int eden_engine_ui_tap(EdenEngine *engine, int x, int y);
EDEN_FFI_API void eden_engine_ui_move(EdenEngine *engine, int x, int y);
EDEN_FFI_API void eden_engine_ui_scroll(EdenEngine *engine, int delta);
EDEN_FFI_API void eden_engine_ui_swipe_up(EdenEngine *engine);
EDEN_FFI_API void eden_engine_ui_two_finger_tap(EdenEngine *engine);
EDEN_FFI_API const char *eden_engine_ui_slots_json(EdenEngine *engine);
EDEN_FFI_API int eden_engine_ui_save_slot(EdenEngine *engine, uint32_t index);
EDEN_FFI_API int eden_engine_ui_load_slot(EdenEngine *engine, uint32_t index);
EDEN_FFI_API int eden_engine_set_save_dir(EdenEngine *engine, const char *path);
EDEN_FFI_API void eden_engine_set_video_enabled(EdenEngine *engine, int enabled);
EDEN_FFI_API int eden_engine_movie_active(EdenEngine *engine);
EDEN_FFI_API int eden_engine_open_edition(EdenEngine *engine, int edition);
EDEN_FFI_API void eden_engine_save_unlocks(EdenEngine *engine);
EDEN_FFI_API int eden_engine_quit_requested(EdenEngine *engine);

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* EDEN_FFI_H */
