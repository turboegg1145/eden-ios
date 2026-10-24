//! Desktop validation CLI for the pure-Rust `eden*` runtime.
//!
//! Every command operates on the retail `.paz` archives. There are no
//! synthetic assets, no recorded traces and no mocked decoders.

mod serve;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use eden_ffi::Runtime;
use eden_paz::{encode_png, Edition, PazArchive, RgbaImage};
use eden_render::ui::UiScreen;
use eden_render::{MessageOverlay, Scene, SoftwareRenderer};
use eden_video::avi::AviFile;
use eden_video::ffmpeg;
use eden_script::{parse_script_bytes, ScriptCommand, ScriptEncoding, VmEvent, VmState};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const DEFAULT_GAME_DIR: &str = "/mnt/f/gal/【PC硬盘】【官中】eden";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("eden: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<(), String> {
    let Some(command) = args.first().map(String::as_str) else {
        print_help();
        return Ok(());
    };
    let rest = &args[1..];
    match command {
        "verify" => cmd_verify(rest),
        "trace" => cmd_trace(rest),
        "shot" | "screenshot" => cmd_shot(rest),
        "list" => cmd_list(rest),
        "catalog" => cmd_catalog(rest),
        "audio" => cmd_audio(rest),
        "movie" => cmd_movie(rest),
        "ui" => cmd_ui(rest),
        "display" => cmd_display(rest),
        "walkthrough" => cmd_walkthrough(rest),
        "serve" => serve::run(rest),
        "help" | "-h" | "--help" => {
            print_help();
            Ok(())
        }
        other => Err(format!("unknown command {other:?}; run `eden help`")),
    }
}

fn print_help() {
    println!(
        r#"eden — pure-Rust Musica runtime validation CLI

USAGE:
  eden verify [--game DIR] [--json PATH] [--skip-samples]
  eden trace  [--game DIR] [--script NAME] [--frames N] [--json PATH] [--pm] [--events]
  eden shot   [--game DIR] [--out DIR] [--max-frames N]
  eden list   <archive.paz> [--filter TEXT]
  eden catalog [--game DIR] [--out PATH] [--extract DIR]
  eden audio  [--game DIR] [--voice NAME] [--json PATH]
  eden movie  [--game DIR] [--out DIR] [--time SECONDS] [--all]
  eden ui     [--game DIR] [--out DIR] [--frames N]
  eden display [--game DIR] [--out DIR]
  eden walkthrough [--game DIR] [--json PATH] [--max-frames N] [--edition main|pm]
  eden serve  [--game DIR] [--bind 127.0.0.1:8787] [--fps N] [--save-dir DIR] [--pm] [--open]

DEFAULTS:
  game dir   $EDEN_GAME_DIR or {DEFAULT_GAME_DIR}
  verify     writes reports/verify.json
  trace      A001.sc, 4000 frames, writes reports/trace_<script>.json
  shot       writes reports/screenshots/*.png + report.json

The CLI reads the retail .paz archives directly; nothing is converted.
"#
    );
}

#[derive(Default)]
struct Opts {
    values: HashMap<String, String>,
    flags: HashSet<String>,
    positional: Vec<String>,
}

impl Opts {
    fn parse(args: &[String]) -> Self {
        let mut opts = Self::default();
        let mut i = 0;
        while i < args.len() {
            let arg = &args[i];
            if let Some(rest) = arg.strip_prefix("--") {
                if let Some((key, value)) = rest.split_once('=') {
                    opts.values.insert(key.to_string(), value.to_string());
                } else if i + 1 < args.len() && !args[i + 1].starts_with("--") && !is_flag(rest) {
                    opts.values.insert(rest.to_string(), args[i + 1].clone());
                    i += 1;
                } else {
                    opts.flags.insert(rest.to_string());
                }
            } else {
                opts.positional.push(arg.clone());
            }
            i += 1;
        }
        opts
    }

    fn value(&self, name: &str) -> Option<&str> {
        self.values.get(name).map(String::as_str)
    }

    fn flag(&self, name: &str) -> bool {
        self.flags.contains(name)
    }

    fn number(&self, name: &str, default: usize) -> Result<usize, String> {
        match self.value(name) {
            Some(value) => value
                .parse()
                .map_err(|_| format!("--{name} expects a number, got {value:?}")),
            None => Ok(default),
        }
    }
}

fn is_flag(name: &str) -> bool {
    matches!(name, "pm" | "events" | "skip-samples" | "quick" | "open")
}

fn game_dir(opts: &Opts) -> PathBuf {
    opts.value("game")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("EDEN_GAME_DIR").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from(DEFAULT_GAME_DIR))
}

fn pm_dir(game: &Path) -> PathBuf {
    let nested = game.join("eden PLUS MOSAIC");
    if nested.exists() {
        nested
    } else {
        game.to_path_buf()
    }
}

fn ensure_parent(path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

fn sha256(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex(&hasher.finalize())
}

fn write_json(path: &Path, value: &Value) -> Result<(), String> {
    ensure_parent(path)?;
    let text = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn encoding_for(archive: &PazArchive) -> ScriptEncoding {
    match archive.edition() {
        Edition::Main => ScriptEncoding::Gbk,
        Edition::PlusMosaic => ScriptEncoding::Cp932,
    }
}

// ---------------------------------------------------------------------------
// verify
// ---------------------------------------------------------------------------

#[derive(Default)]
struct ArchiveStats {
    entries: usize,
    png: usize,
    ani: usize,
    ani_frames: usize,
    ogg: usize,
    wav: usize,
    avi: usize,
    psd: usize,
    sc: usize,
    txt: usize,
    other: usize,
    decoded_png: usize,
    decoded_audio: usize,
    unpacked_bytes: u64,
}

impl ArchiveStats {
    fn to_json(&self) -> Value {
        json!({
            "entries": self.entries,
            "png": self.png,
            "ani": self.ani,
            "ani_frames": self.ani_frames,
            "ogg": self.ogg,
            "wav": self.wav,
            "avi": self.avi,
            "psd": self.psd,
            "sc": self.sc,
            "txt": self.txt,
            "other": self.other,
            "decoded_png": self.decoded_png,
            "decoded_audio": self.decoded_audio,
            "unpacked_bytes": self.unpacked_bytes,
        })
    }
}

struct VerifyOptions {
    decode_png: usize,
    decode_audio: usize,
    parse_ani: bool,
}

fn sniff(ext: &str, prefix: &[u8]) -> Result<(), String> {
    let ok = match ext {
        "png" => prefix.starts_with(b"\x89PNG\r\n\x1a\n"),
        "ogg" => prefix.starts_with(b"OggS"),
        "wav" => prefix.len() >= 12 && &prefix[..4] == b"RIFF" && &prefix[8..12] == b"WAVE",
        "avi" => prefix.len() >= 12 && &prefix[..4] == b"RIFF" && &prefix[8..12] == b"AVI ",
        "psd" => prefix.starts_with(b"8BPS"),
        _ => true,
    };
    if ok {
        Ok(())
    } else {
        Err(format!(
            "bad {ext} magic: {:02x?}",
            &prefix[..prefix.len().min(16)]
        ))
    }
}

fn verify_archive(
    path: &Path,
    label: &str,
    options: &VerifyOptions,
) -> Result<(Value, Vec<String>, ArchiveStats), String> {
    let archive = PazArchive::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let file_len = fs::metadata(path)
        .map_err(|e| format!("{}: {e}", path.display()))?
        .len();
    let mut stats = ArchiveStats::default();
    let mut failures = Vec::new();
    let entries = archive.entries();
    let stride = if options.decode_png == 0 {
        usize::MAX
    } else {
        (entries.len() / options.decode_png.max(1)).max(1)
    };

    for (index, entry) in entries.iter().enumerate() {
        stats.entries += 1;
        stats.unpacked_bytes += u64::from(entry.unpacked_size);
        if entry.aligned_size < entry.size {
            failures.push(format!(
                "{label}: {} aligned_size {} < size {}",
                entry.name, entry.aligned_size, entry.size
            ));
        }
        if !archive.is_video_archive() && entry.aligned_size % 8 != 0 {
            failures.push(format!(
                "{label}: {} aligned_size {} is not a multiple of 8",
                entry.name, entry.aligned_size
            ));
        }
        if entry.offset.saturating_add(u64::from(entry.aligned_size)) > file_len {
            failures.push(format!(
                "{label}: {} data range {}..{} is past EOF {file_len}",
                entry.name,
                entry.offset,
                entry.offset + u64::from(entry.aligned_size)
            ));
        }
        if entry.size > entry.unpacked_size {
            failures.push(format!(
                "{label}: {} size {} > unpacked_size {}",
                entry.name, entry.size, entry.unpacked_size
            ));
        }

        let ext = entry.extension();
        match ext.as_str() {
            "png" => {
                stats.png += 1;
                match archive.read_entry_prefix(entry, 32) {
                    Ok(prefix) => {
                        if let Err(e) = sniff("png", &prefix) {
                            failures.push(format!("{label}: {}: {e}", entry.name));
                        } else if prefix.len() >= 24 {
                            let width = u32::from_be_bytes(prefix[16..20].try_into().unwrap());
                            let height = u32::from_be_bytes(prefix[20..24].try_into().unwrap());
                            if width == 0 || height == 0 {
                                failures
                                    .push(format!("{label}: {} has zero dimensions", entry.name));
                            }
                        }
                    }
                    Err(e) => failures.push(format!("{label}: {}: {e}", entry.name)),
                }
                if stats.decoded_png < options.decode_png && index % stride == 0 {
                    match archive.decode_png(&entry.name) {
                        Ok(image) => {
                            stats.decoded_png += 1;
                            if image.width == 0 || image.height == 0 {
                                failures.push(format!(
                                    "{label}: {} decoded to an empty image",
                                    entry.name
                                ));
                            }
                        }
                        Err(e) => {
                            failures.push(format!("{label}: {} PNG decode failed: {e}", entry.name))
                        }
                    }
                }
            }
            "ani" => {
                stats.ani += 1;
                if options.parse_ani {
                    match archive.parse_ani(&entry.name) {
                        Ok(ani) => {
                            stats.ani_frames += ani.frame_count();
                            let stem = entry.name[..entry.name.len() - 4].to_string();
                            if archive.entry(&format!("{stem}.png")).is_none() {
                                failures
                                    .push(format!("{label}: {} has no same-named PNG", entry.name));
                            }
                        }
                        Err(e) => {
                            failures.push(format!("{label}: {} ANI parse failed: {e}", entry.name))
                        }
                    }
                }
            }
            "ogg" | "wav" => {
                if ext == "ogg" {
                    stats.ogg += 1;
                } else {
                    stats.wav += 1;
                }
                match archive.read_entry_prefix(entry, 16) {
                    Ok(prefix) => {
                        if let Err(e) = sniff(&ext, &prefix) {
                            failures.push(format!("{label}: {}: {e}", entry.name));
                        }
                    }
                    Err(e) => failures.push(format!("{label}: {}: {e}", entry.name)),
                }
                if stats.decoded_audio < options.decode_audio {
                    match eden_audio::decode_paz_entry(&archive, &entry.name) {
                        Ok(clip) => {
                            stats.decoded_audio += 1;
                            if clip.frames() == 0 || clip.peak() <= 0.0 {
                                failures
                                    .push(format!("{label}: {} decoded to silence", entry.name));
                            }
                        }
                        Err(e) => failures
                            .push(format!("{label}: {} audio decode failed: {e}", entry.name)),
                    }
                }
            }
            "avi" => {
                stats.avi += 1;
                match archive.read_entry_prefix(entry, 16) {
                    Ok(prefix) => {
                        if let Err(e) = sniff("avi", &prefix) {
                            failures.push(format!("{label}: {}: {e}", entry.name));
                        }
                    }
                    Err(e) => failures.push(format!("{label}: {}: {e}", entry.name)),
                }
            }
            "psd" => {
                stats.psd += 1;
                match archive.read_entry_prefix(entry, 8) {
                    Ok(prefix) => {
                        if let Err(e) = sniff("psd", &prefix) {
                            failures.push(format!("{label}: {}: {e}", entry.name));
                        }
                    }
                    Err(e) => failures.push(format!("{label}: {}: {e}", entry.name)),
                }
            }
            "sc" => stats.sc += 1,
            "txt" => {
                stats.txt += 1;
                if let Err(e) = archive.read_entry_obj(entry) {
                    failures.push(format!("{label}: {}: {e}", entry.name));
                }
            }
            _ => stats.other += 1,
        }
    }

    Ok((
        json!({
            "label": label,
            "path": path.display().to_string(),
            "ident": archive.ident(),
            "edition": format!("{:?}", archive.edition()),
            "slot": archive.slot(),
            "file_bytes": file_len,
            "stats": stats.to_json(),
            "ok": failures.is_empty(),
        }),
        failures,
        stats,
    ))
}

#[derive(Default, Clone)]
struct ScriptSummary {
    count: usize,
    code_lines: usize,
    commands: BTreeMap<String, usize>,
    errors: Vec<String>,
    label_failures: Vec<String>,
}

impl ScriptSummary {
    fn add(&mut self, other: &ScriptSummary) {
        self.count += other.count;
        self.code_lines += other.code_lines;
        for (name, count) in &other.commands {
            *self.commands.entry(name.clone()).or_insert(0) += count;
        }
        self.errors.extend(other.errors.iter().cloned());
        self.label_failures
            .extend(other.label_failures.iter().cloned());
    }

    fn to_json(&self, label: &str) -> Value {
        json!({
            "label": label,
            "scripts": self.count,
            "code_lines": self.code_lines,
            "commands": self.commands,
            "errors": self.errors,
            "label_failures": self.label_failures,
        })
    }
}

fn verify_scripts(path: &Path, label: &str) -> Result<ScriptSummary, String> {
    let archive = PazArchive::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let encoding = encoding_for(&archive);
    let mut summary = ScriptSummary::default();
    for entry in archive.entries() {
        if !entry.name.to_ascii_lowercase().ends_with(".sc") {
            continue;
        }
        let bytes = archive
            .read_entry_obj(entry)
            .map_err(|e| format!("{label}: {}: {e}", entry.name))?;
        let script = parse_script_bytes(entry.name.clone(), &bytes, encoding)
            .map_err(|e| format!("{label}: {}: {e}", entry.name))?;
        summary.count += 1;
        summary.code_lines += script.stats.code_lines;
        for error in &script.errors {
            summary.errors.push(format!(
                "{label}: {}:{}: {}",
                entry.name, error.line, error.message
            ));
        }
        for command in &script.commands {
            *summary
                .commands
                .entry(command.name().to_string())
                .or_insert(0) += 1;
            let target = match command {
                ScriptCommand::If(i) => Some(&i.label),
                ScriptCommand::Goto(g) => Some(&g.label),
                _ => None,
            };
            if let Some(target) = target {
                if !script.labels.contains_key(target) {
                    summary
                        .label_failures
                        .push(format!("{label}: {}: missing label {target:?}", entry.name));
                }
            }
        }
    }
    Ok(summary)
}

const EXPECTED_COMMANDS: &[(&str, usize)] = &[
    (".message", 22_048),
    (".stage", 7_730),
    (".transition", 7_729),
    (".panel", 948),
    (".playBGM", 552),
    (".playSE", 430),
    (".wait", 351),
    (".pragma", 142),
    (".label", 78),
    (".if", 74),
    (".endScroll", 52),
    (".chain", 42),
    (".playSE2", 38),
    (".setGlobal", 30),
    (".vscroll", 25),
    (".end", 22),
    (".set", 22),
    (".playVoice", 18),
    (".scroll", 18),
    (".movie", 11),
    (".hscroll", 9),
    (".goto", 8),
    (".shakeScreen", 8),
    (".effect", 4),
];

fn cmd_verify(args: &[String]) -> Result<(), String> {
    let opts = Opts::parse(args);
    let game = game_dir(&opts);
    let report_path = PathBuf::from(opts.value("json").unwrap_or("reports/verify.json"));
    let mut archives = Vec::new();
    let mut failures = Vec::new();
    let mut script_reports = Vec::new();
    let mut aggregate = ScriptSummary::default();

    let mut specs: Vec<(String, PathBuf, VerifyOptions)> = Vec::new();
    if !opts.flag("skip-samples") {
        let samples = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../samples");
        for (label, path) in [
            ("samples/main/scr.paz", samples.join("main/scr.paz")),
            ("samples/main/st.paz", samples.join("main/st.paz")),
            ("samples/pm/scr.paz", samples.join("pm/scr.paz")),
            ("samples/pm/st.paz", samples.join("pm/st.paz")),
        ] {
            if path.exists() {
                specs.push((
                    label.to_string(),
                    path,
                    VerifyOptions {
                        decode_png: 0,
                        decode_audio: 0,
                        parse_ani: false,
                    },
                ));
            } else {
                failures.push(format!("{label}: missing sample archive"));
            }
        }
    }

    for (name, options) in [
        (
            "scr",
            VerifyOptions {
                decode_png: 0,
                decode_audio: 0,
                parse_ani: false,
            },
        ),
        (
            "bg",
            VerifyOptions {
                decode_png: 32,
                decode_audio: 0,
                parse_ani: true,
            },
        ),
        (
            "sys",
            VerifyOptions {
                decode_png: 24,
                decode_audio: 0,
                parse_ani: true,
            },
        ),
        (
            "st",
            VerifyOptions {
                decode_png: 0,
                decode_audio: 0,
                parse_ani: false,
            },
        ),
        (
            "bgm",
            VerifyOptions {
                decode_png: 0,
                decode_audio: 2,
                parse_ani: false,
            },
        ),
        (
            "se",
            VerifyOptions {
                decode_png: 0,
                decode_audio: 3,
                parse_ani: false,
            },
        ),
        (
            "voice",
            VerifyOptions {
                decode_png: 0,
                decode_audio: 3,
                parse_ani: false,
            },
        ),
        (
            "mov",
            VerifyOptions {
                decode_png: 0,
                decode_audio: 0,
                parse_ani: false,
            },
        ),
    ] {
        let path = game.join(format!("{name}.paz"));
        if path.exists() {
            specs.push((format!("game/{name}.paz"), path, options));
        } else if matches!(name, "scr" | "bg" | "sys") {
            failures.push(format!("game/{name}.paz: missing required archive"));
        }
    }

    for (label, path, options) in &specs {
        match verify_archive(path, label, options) {
            Ok((report, mut errors, _stats)) => {
                archives.push(report);
                failures.append(&mut errors);
            }
            Err(e) => failures.push(format!("{label}: {e}")),
        }
    }

    // Scripts are parsed from the archives, not from extracted copies.
    let mut script_inputs = Vec::new();
    if !opts.flag("skip-samples") {
        let samples = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../samples");
        script_inputs.push(("samples/main/scr.paz", samples.join("main/scr.paz"), false));
        script_inputs.push(("samples/pm/scr.paz", samples.join("pm/scr.paz"), false));
    }
    script_inputs.push(("game/scr.paz", game.join("scr.paz"), true));
    let pm = pm_dir(&game);
    if pm != game {
        script_inputs.push(("game/eden PLUS MOSAIC/scr.paz", pm.join("scr.paz"), true));
    }

    for (label, path, contributes) in script_inputs {
        if !path.exists() {
            if contributes {
                failures.push(format!("{label}: missing script archive"));
            }
            continue;
        }
        match verify_scripts(&path, label) {
            Ok(summary) => {
                if contributes {
                    aggregate.add(&summary);
                }
                script_reports.push(summary.to_json(label));
            }
            Err(e) => failures.push(e),
        }
    }

    let aggregate_checked = aggregate.count > 0;
    if aggregate_checked {
        if aggregate.code_lines != 40_397 {
            failures.push(format!(
                "aggregate code_lines {} != 40397",
                aggregate.code_lines
            ));
        }
        for (name, expected) in EXPECTED_COMMANDS {
            let got = aggregate.commands.get(*name).copied().unwrap_or(0);
            if got != *expected {
                failures.push(format!("aggregate {name} count {got} != {expected}"));
            }
        }
        if aggregate.count != 58 {
            failures.push(format!(
                "aggregate script count {} != 58 (main 23 + PM 35)",
                aggregate.count
            ));
        }
    }
    failures.extend(aggregate.errors.iter().cloned());
    failures.extend(aggregate.label_failures.iter().cloned());

    let total_entries: usize = archives
        .iter()
        .filter_map(|a| a["stats"]["entries"].as_u64())
        .map(|n| n as usize)
        .sum();
    let total_unpacked: u64 = archives
        .iter()
        .filter_map(|a| a["stats"]["unpacked_bytes"].as_u64())
        .sum();

    let report = json!({
        "generated_unix": now_unix(),
        "game_dir": game.display().to_string(),
        "archives": archives,
        "scripts": script_reports,
        "aggregate": aggregate.to_json("game main + PM"),
        "aggregate_checked": aggregate_checked,
        "totals": {
            "archives": specs.len(),
            "entries": total_entries,
            "unpacked_bytes": total_unpacked,
        },
        "failures": failures,
        "ok": failures.is_empty(),
    });
    write_json(&report_path, &report)?;

    println!(
        "verify: {} archives, {} entries, {:.1} MiB unpacked",
        specs.len(),
        total_entries,
        total_unpacked as f64 / (1024.0 * 1024.0)
    );
    for archive in &archives {
        let stats = &archive["stats"];
        println!(
            "  {:<34} {:>5} entries  png {:>5}  ani {:>4}  ogg {:>4}  ok={}",
            archive["label"].as_str().unwrap_or("?"),
            stats["entries"],
            stats["png"],
            stats["ani"],
            stats["ogg"],
            archive["ok"]
        );
    }
    println!(
        "scripts: {} parsed, {} code lines, {} command kinds",
        aggregate.count,
        aggregate.code_lines,
        aggregate.commands.len()
    );
    if failures.is_empty() {
        println!("verify: OK (0 failures)");
    } else {
        println!("verify: {} FAILURES", failures.len());
        for failure in failures.iter().take(20) {
            println!("  - {failure}");
        }
        if failures.len() > 20 {
            println!("  ... {} more", failures.len() - 20);
        }
    }
    println!("report: {}", report_path.display());
    if failures.is_empty() {
        Ok(())
    } else {
        Err(format!("{} verification failures", failures.len()))
    }
}

// ---------------------------------------------------------------------------
// trace
// ---------------------------------------------------------------------------

fn event_name(event: &VmEvent) -> &'static str {
    match event {
        VmEvent::Message { .. } => "message",
        VmEvent::Stage { .. } => "stage",
        VmEvent::Transition { .. } => "transition",
        VmEvent::Panel { .. } => "panel",
        VmEvent::PlayBgm { .. } => "playBGM",
        VmEvent::PlaySe { .. } => "playSE",
        VmEvent::PlayVoice { .. } => "playVoice",
        VmEvent::Wait { .. } => "wait",
        VmEvent::Scroll { .. } => "scroll",
        VmEvent::EndScroll { .. } => "endScroll",
        VmEvent::Movie { .. } => "movie",
        VmEvent::ShakeScreen { .. } => "shakeScreen",
        VmEvent::Effect { .. } => "effect",
        VmEvent::Chain { .. } => "chain",
        VmEvent::Goto { .. } => "goto",
        VmEvent::Set { .. } => "set",
        VmEvent::Pragma { .. } => "pragma",
        VmEvent::TransitionEnd => "transitionEnd",
        VmEvent::ScrollEnd => "scrollEnd",
        VmEvent::MovieEnd { .. } => "movieEnd",
        VmEvent::End { .. } => "end",
        VmEvent::Compat { .. } => "compat",
        VmEvent::Error { .. } => "error",
    }
}

fn state_name(state: &VmState) -> &'static str {
    match state {
        VmState::Running => "Running",
        VmState::WaitFrames { .. } => "WaitFrames",
        VmState::WaitClick => "WaitClick",
        VmState::WaitScroll { .. } => "WaitScroll",
        VmState::WaitTransition { .. } => "WaitTransition",
        VmState::WaitMovie { .. } => "WaitMovie",
        VmState::WaitBgm { .. } => "WaitBgm",
        VmState::Ended => "Ended",
    }
}

fn cmd_trace(args: &[String]) -> Result<(), String> {
    let opts = Opts::parse(args);
    let mut game = game_dir(&opts);
    if opts.flag("pm") {
        game = pm_dir(&game);
    }
    let script = opts.value("script").unwrap_or("A001.sc");
    let max_frames = opts.number("frames", 4000)?;
    let default_report = format!(
        "reports/trace_{}.json",
        script.trim_end_matches(".sc").replace(['/', '\\'], "_")
    );
    let report_path = PathBuf::from(opts.value("json").unwrap_or(&default_report));

    let mut runtime = Runtime::open(&game, None).map_err(|e| format!("{}: {e}", game.display()))?;
    runtime.set_auto_advance(true);
    runtime
        .start_script(script)
        .map_err(|e| format!("{script}: {e}"))?;

    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut errors = Vec::new();
    let mut stage_timeline = Vec::new();
    let mut messages = Vec::new();
    let mut snapshots = Vec::new();
    let mut last_stage: Option<(String, i32, i32, i32, i32)> = None;
    let mut last_message: Option<i64> = None;
    let mut frames = 0usize;
    let mut ended = false;

    for frame in 1..=max_frames {
        runtime
            .tick_vm()
            .map_err(|e| format!("frame {frame}: {e}"))?;
        frames = frame;
        for event in runtime.events() {
            *counts.entry(event_name(event).to_string()).or_insert(0) += 1;
            match event {
                VmEvent::Error { message, line } => {
                    errors.push(format!("frame {frame} line {line}: {message}"));
                }
                VmEvent::Stage {
                    image, x, y, line, ..
                } => {
                    let view = runtime.vm().map(|vm| vm.view()).unwrap_or_default();
                    let key = (image.clone(), *x, *y, view.x, view.y);
                    if last_stage.as_ref() != Some(&key) {
                        stage_timeline.push(json!({
                            "frame": frame,
                            "line": line,
                            "image": image,
                            "x": x,
                            "y": y,
                            "view_x": view.x,
                            "view_y": view.y,
                        }));
                        last_stage = Some(key);
                    }
                }
                VmEvent::Message {
                    id,
                    voice,
                    speaker,
                    line,
                    ..
                } => {
                    if last_message != Some(*id) {
                        messages.push(json!({
                            "frame": frame,
                            "line": line,
                            "id": id,
                            "voice": voice,
                            "speaker": speaker,
                        }));
                        last_message = Some(*id);
                    }
                }
                _ => {}
            }
        }
        if opts.flag("events") {
            for event in runtime.events() {
                println!(
                    "{frame}: {}",
                    serde_json::to_string(event).unwrap_or_default()
                );
            }
        }
        if frame % 60 == 0 {
            let vm = runtime.vm().unwrap();
            snapshots.push(json!({
                "frame": frame,
                "pc": vm.pc(),
                "state": state_name(vm.state()),
                "script": vm.script_name(),
                "stage": vm.stage(),
                "view": vm.view(),
                "panel": vm.panel(),
                "message_id": vm.message().map(|m| m.id),
                "scroll": vm.scroll(),
                "transition": vm.transition(),
            }));
        }
        if matches!(runtime.vm().map(|vm| vm.state()), Some(VmState::Ended)) {
            ended = true;
            break;
        }
    }

    let (save_json_ok, save_binary_ok) = {
        let vm = runtime.vm().ok_or("no VM after trace")?;
        let save = vm.save();
        let json_ok =
            eden_script::SaveState::from_json(&save.to_json().map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?
                == save;
        let binary_ok =
            eden_script::SaveState::from_binary(&save.to_binary().map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?
                == save;
        (json_ok, binary_ok)
    };

    let save_path = std::env::temp_dir().join("eden_trace_roundtrip.json");
    runtime
        .save_to(&save_path)
        .map_err(|e| format!("save: {e}"))?;
    let before = runtime.state_json().map_err(|e| e.to_string())?;
    runtime
        .load_from(&save_path)
        .map_err(|e| format!("load: {e}"))?;
    let after = runtime.state_json().map_err(|e| e.to_string())?;
    let reload_equal = before == after;
    let mut post_load_error = None;
    for _ in 0..120 {
        if let Err(e) = runtime.tick_vm() {
            post_load_error = Some(e.to_string());
            break;
        }
    }
    if post_load_error.is_none() {
        post_load_error = runtime.last_error().map(ToOwned::to_owned);
    }
    let _ = fs::remove_file(save_path);

    let vm = runtime.vm().ok_or("no VM after trace")?;
    let final_state = json!({
        "frames": frames,
        "ended": ended,
        "script": vm.script_name(),
        "pc": vm.pc(),
        "state": state_name(vm.state()),
        "stage": vm.stage(),
        "view": vm.view(),
        "panel": vm.panel(),
        "message_id": vm.message().map(|m| m.id),
        "bgm": vm.bgm(),
        "last_error": vm.last_error().map(|e| e.to_string()),
    });
    let report = json!({
        "generated_unix": now_unix(),
        "game_dir": game.display().to_string(),
        "script": script,
        "frames": frames,
        "event_counts": counts,
        "stage_timeline": stage_timeline,
        "messages": messages,
        "snapshots": snapshots,
        "errors": errors,
        "save_roundtrip": {
            "json": save_json_ok,
            "binary": save_binary_ok,
            "runtime_reload": reload_equal,
            "post_load_ticks": 120,
            "post_load_error": post_load_error,
        },
        "final": final_state,
        "ok": errors.is_empty()
            && save_json_ok
            && save_binary_ok
            && reload_equal
            && post_load_error.is_none(),
    });
    write_json(&report_path, &report)?;

    println!(
        "trace: {script} -> {frames} frames, state={}, pc={}, ended={}",
        final_state["state"].as_str().unwrap_or("?"),
        final_state["pc"],
        ended
    );
    println!(
        "events: {} kinds, stages={}, messages={}, errors={}",
        counts.len(),
        stage_timeline.len(),
        messages.len(),
        errors.len()
    );
    println!(
        "save roundtrip: json={save_json_ok} binary={save_binary_ok} reload={reload_equal} post_load_error={:?}",
        post_load_error
    );
    if !errors.is_empty() {
        for error in errors.iter().take(10) {
            println!("  - {error}");
        }
    }
    println!("report: {}", report_path.display());
    if report["ok"].as_bool().unwrap_or(false) {
        Ok(())
    } else {
        Err("trace validation failed".into())
    }
}

// ---------------------------------------------------------------------------
// shot
// ---------------------------------------------------------------------------

fn save_frame(dir: &Path, name: &str, frame: &RgbaImage) -> Result<Value, String> {
    fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let path = dir.join(name);
    let png = encode_png(frame).map_err(|e| e.to_string())?;
    fs::write(&path, &png).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(json!({
        "file": path.display().to_string(),
        "width": frame.width,
        "height": frame.height,
        "png_bytes": png.len(),
        "sha256_png": sha256(&png),
        "sha256_rgba": sha256(&frame.data),
    }))
}

/// Render the current UI without the debug cursor and save the frame.
fn snap_ui(runtime: &mut Runtime, dir: &Path, name: &str) -> Result<Value, String> {
    runtime.ui_mut().cursor_visible = false;
    runtime.render_now();
    save_frame(dir, name, runtime.frame())
}

fn message_text(archive: &PazArchive, script: &str, id: i64) -> Result<String, String> {
    let bytes = archive
        .read_entry(script)
        .map_err(|e| format!("{script}: {e}"))?;
    let parsed = parse_script_bytes(script, &bytes, encoding_for(archive))
        .map_err(|e| format!("{script}: {e}"))?;
    for command in &parsed.commands {
        if let ScriptCommand::Message(message) = command {
            if message.id == id {
                let mut text = String::new();
                if let Some(speaker) = &message.speaker {
                    text.push_str(speaker);
                    text.push('\n');
                }
                text.push_str(&message.text.plain_text());
                return Ok(text);
            }
        }
    }
    Err(format!("{script}: message {id} not found"))
}

fn panel_image(runtime: &mut Runtime, name: &str) -> Result<Arc<RgbaImage>, String> {
    runtime.image(name).map_err(|e| e.to_string())
}

fn cmd_shot(args: &[String]) -> Result<(), String> {
    let opts = Opts::parse(args);
    let game = game_dir(&opts);
    let out_dir = PathBuf::from(opts.value("out").unwrap_or("reports/screenshots"));
    let max_frames = opts.number("max-frames", 12000)?;
    let mut runtime = Runtime::open(&game, None).map_err(|e| format!("{}: {e}", game.display()))?;
    // Advance messages one click at a time so each text box is observable.
    runtime.set_auto_advance(false);
    let mut frames = Vec::new();
    let mut notes = Vec::new();

    // --- 1. Actual A001 trace: long stage + .ani + vscroll + text ----------
    runtime
        .start_script("A001.sc")
        .map_err(|e| format!("A001.sc: {e}"))?;
    let mut mid_saved = false;
    let mut text_saved = false;
    let mut text_590 = String::new();
    for frame in 1..=max_frames {
        runtime
            .tick_vm()
            .map_err(|e| format!("A001 frame {frame}: {e}"))?;
        let (image, view_y, message_id, panel_mode) = {
            let vm = runtime.vm().unwrap();
            (
                vm.stage().image.clone(),
                vm.view().y,
                vm.message().map(|m| m.id),
                vm.panel().mode,
            )
        };
        if !mid_saved && image == "ev007a01.png" && (200..600).contains(&view_y) {
            runtime.render_now();
            let entry = save_frame(&out_dir, "01_a001_vscroll_mid_ani.png", runtime.frame())?;
            frames.push(entry);
            let ani_frames = runtime
                .scene()
                .layers
                .iter()
                .find(|l| l.name == "stage")
                .and_then(|l| l.ani.as_ref().map(|a| a.ani.frame_count()))
                .unwrap_or(0);
            notes.push(format!(
                "A001 frame {frame}: stage=ev007a01.png view_y={view_y} .ani={ani_frames} frames"
            ));
            mid_saved = true;
        }
        if image == "ev007a01.png" && message_id == Some(590) && panel_mode != 0 {
            // Wait until the incoming stage is mostly revealed so the PNG shows
            // the long image rather than the outgoing frame at progress 0.
            let transition_progress = runtime
                .vm()
                .and_then(|vm| vm.transition())
                .map(|t| {
                    if t.total_frames == 0 {
                        1.0
                    } else {
                        t.elapsed as f32 / t.total_frames as f32
                    }
                })
                .unwrap_or(1.0);
            if transition_progress < 0.8 {
                continue;
            }
            runtime.render_now();
            let entry = save_frame(&out_dir, "02_a001_vscroll_text_panel.png", runtime.frame())?;
            frames.push(entry);
            notes.push(format!(
                "A001 frame {frame}: stage=ev007a01.png view_y={view_y} message=590 panel={panel_mode} transition={transition_progress:.2}"
            ));
            if let Some(message) = runtime.vm().and_then(|vm| vm.message()) {
                if let Some(speaker) = &message.speaker {
                    text_590.push_str(speaker);
                    text_590.push('\n');
                }
                text_590.push_str(&message.text.plain_text());
            }
            text_saved = true;
            break;
        }
        if matches!(runtime.vm_state(), Some(VmState::WaitClick)) {
            runtime.click();
        }
    }
    if !mid_saved || !text_saved {
        return Err(format!(
            "A001 did not reach the ev007a01 long-stage frames (mid={mid_saved}, text={text_saved})"
        ));
    }

    // --- 2. Actual B006 trace: rule-mask transition -----------------------
    runtime
        .start_script("B006.sc")
        .map_err(|e| format!("B006.sc: {e}"))?;
    let mut transition_saved = false;
    for frame in 1..=max_frames {
        runtime
            .tick_vm()
            .map_err(|e| format!("B006 frame {frame}: {e}"))?;
        let transition = runtime.vm().and_then(|vm| vm.transition()).cloned();
        if let Some(t) = transition {
            if t.kind == 1 && t.elapsed > 0 && t.elapsed + 2 >= t.total_frames / 2 {
                runtime.render_now();
                let entry = save_frame(&out_dir, "03_b006_mask_transition.png", runtime.frame())?;
                frames.push(entry);
                let vm = runtime.vm().unwrap();
                notes.push(format!(
                    "B006 frame {frame}: transition kind={} layer={:?} {}/{} stage={} view_y={}",
                    t.kind,
                    t.layer,
                    t.elapsed,
                    t.total_frames,
                    vm.stage().image,
                    vm.view().y
                ));
                transition_saved = true;
                break;
            }
        }
        if matches!(runtime.vm_state(), Some(VmState::WaitClick)) {
            runtime.click();
        }
    }
    if !transition_saved {
        return Err("B006 did not reach a rule-mask transition".into());
    }

    // --- 3. Real .ani differential compositing ---------------------------
    let base = runtime
        .image("ev_add02_up06a.png")
        .map_err(|e| e.to_string())?;
    let ani = runtime
        .ani("ev_add02_up06a.ani")
        .map_err(|e| e.to_string())?;
    let mut renderer = SoftwareRenderer::new(1024, 640).with_system_font();
    let mut scene = Scene::new(1024, 640);
    scene.set_stage(base, 0, 0, None);
    let base_frame = renderer.render(&scene);
    let base_entry = save_frame(&out_dir, "04_ani_base.png", &base_frame)?;
    scene.set_stage_ani(ani.clone(), 0, 255);
    let overlay_frame = renderer.render(&scene);
    let overlay_entry = save_frame(&out_dir, "05_ani_overlay.png", &overlay_frame)?;
    let changed = base_frame
        .data
        .chunks_exact(4)
        .zip(overlay_frame.data.chunks_exact(4))
        .filter(|(a, b)| a != b)
        .count();
    if changed == 0 {
        return Err("ev_add02_up06a.ani produced no pixel change".into());
    }
    notes.push(format!(
        "ev_add02_up06a.ani frame 0 changed {changed} pixels ({} frames)",
        ani.frame_count()
    ));
    frames.push(base_entry);
    frames.push(overlay_entry);

    // --- 4. Full-chain composite from real assets ------------------------
    let scr = PazArchive::open(game.join("scr.paz"))
        .map_err(|e| format!("{}: {e}", game.join("scr.paz").display()))?;
    let text_500 = message_text(&scr, "A001.sc", 500)?;
    let ev006 = runtime.image("ev006.png").map_err(|e| e.to_string())?;
    let ev007 = runtime.image("ev007a01.png").map_err(|e| e.to_string())?;
    let ev007_ani = runtime.ani("ev007a01.ani").map_err(|e| e.to_string())?;
    let msg_panel = panel_image(&mut runtime, "msgPanel.png")?;
    let msg_panel_omni = panel_image(&mut runtime, "msgPanel_Omni.png")?;
    let fade_side = panel_image(&mut runtime, "fadeSide.png")?;

    let mut scene = Scene::new(1024, 640);
    scene.set_stage(ev006, 0, 1280, None);
    scene.set_view(0, 970);
    scene.set_panel(true, Some(MessageOverlay::new(text_500)));
    scene.set_panel_image(Some(msg_panel));
    let from_frame = renderer.render(&scene);
    let from_entry = save_frame(&out_dir, "06_full_chain_from.png", &from_frame)?;
    renderer.begin_transition(&mut scene, 1, Some(fade_side), 30);

    scene.set_stage(ev007, 0, 0, None);
    scene.set_view(0, 884);
    scene.set_stage_ani(ev007_ani, 1, 255);
    scene.set_panel(true, Some(MessageOverlay::new(text_590)));
    scene.set_panel_image(Some(msg_panel_omni));
    if let Some(transition) = &mut scene.transition {
        transition.elapsed = 15;
    }
    let composite = renderer.render(&scene);
    let composite_entry = save_frame(&out_dir, "07_full_chain_mask_text.png", &composite)?;
    scene.transition = None;
    let final_frame = renderer.render(&scene);
    let final_entry = save_frame(&out_dir, "08_full_chain_final.png", &final_frame)?;
    frames.push(from_entry);
    frames.push(composite_entry);
    frames.push(final_entry);
    notes.push(
        "full-chain composite: ev006 long stage + view 970 -> ev007a01 view 884 + .ani frame 1 + msgPanel_Omni + fadeSide mask @ 15/30"
            .to_string(),
    );

    let report = json!({
        "generated_unix": now_unix(),
        "game_dir": game.display().to_string(),
        "out_dir": out_dir.display().to_string(),
        "frames": frames,
        "notes": notes,
        "ani_changed_pixels": changed,
        "ok": true,
    });
    write_json(&out_dir.join("report.json"), &report)?;

    println!("shot: {} key frames -> {}", frames.len(), out_dir.display());
    for frame in &frames {
        let name = Path::new(frame["file"].as_str().unwrap_or("?"))
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("?");
        let hash = frame["sha256_rgba"].as_str().unwrap_or("");
        println!(
            "  {:<38} {}x{}  sha256={}",
            name,
            frame["width"],
            frame["height"],
            hash.chars().take(16).collect::<String>()
        );
    }
    for note in &notes {
        println!("  note: {note}");
    }
    println!("report: {}", out_dir.join("report.json").display());
    Ok(())
}


// ---------------------------------------------------------------------------
// movie / ui / walkthrough
// ---------------------------------------------------------------------------

fn extract_movie_entry(archive: &Path, name: &str) -> Result<PathBuf, String> {
    let paz = PazArchive::open(archive).map_err(|e| format!("{}: {e}", archive.display()))?;
    let entry = paz
        .entry(name)
        .ok_or_else(|| format!("{}: missing {name}", archive.display()))?
        .clone();
    let dir = std::env::temp_dir().join("eden-video-cache");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let ident: String = paz
        .ident()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    let path = dir.join(format!("{ident}-{name}"));
    if let Ok(meta) = fs::metadata(&path) {
        if meta.len() == u64::from(entry.unpacked_size) {
            return Ok(path);
        }
    }
    let mut reader = paz.open_entry(&entry).map_err(|e| e.to_string())?;
    let mut file = fs::File::create(&path).map_err(|e| e.to_string())?;
    std::io::copy(&mut reader, &mut file).map_err(|e| e.to_string())?;
    Ok(path)
}

fn cmd_movie(args: &[String]) -> Result<(), String> {
    let opts = Opts::parse(args);
    let game = game_dir(&opts);
    let out_dir = PathBuf::from(opts.value("out").unwrap_or("reports/screenshots"));
    let time: f64 = opts
        .value("time")
        .unwrap_or("2.0")
        .parse()
        .map_err(|_| "--time expects seconds".to_string())?;
    if !ffmpeg::is_available() {
        return Err("ffmpeg is required for the desktop video decoder bridge".into());
    }
    let pm = pm_dir(&game);
    let mut sources: Vec<(&str, PathBuf)> = vec![("main", game.join("mov.paz"))];
    if pm != game && pm.join("mov.paz").exists() {
        sources.push(("pm", pm.join("mov.paz")));
    }
    let mut entries = Vec::new();
    let mut index = 9;
    for (edition, archive) in &sources {
        if !archive.exists() {
            continue;
        }
        for name in ["op.avi", "ed.avi"] {
            let path = match extract_movie_entry(archive, name) {
                Ok(path) => path,
                Err(e) => {
                    eprintln!("skip {edition} {name}: {e}");
                    continue;
                }
            };
            let avi = AviFile::open(&path).map_err(|e| format!("{name}: {e}"))?;
            let clip = avi.decode_audio().map_err(|e| format!("{name} audio: {e}"))?;
            let frame = ffmpeg::decode_frame_at(&path, time, 1024, 640, avi.video.fps)
                .map_err(|e| format!("{name} frame: {e}"))?;
            let lit = frame
                .data
                .chunks_exact(4)
                .filter(|p| p[0] > 8 || p[1] > 8 || p[2] > 8)
                .count();
            if lit < 500 {
                return Err(format!("{edition} {name}: frame at {time}s is blank ({lit} px)"));
            }
            let file = if *edition == "main" {
                format!("{index:02}_movie_{}.png", name.trim_end_matches(".avi"))
            } else {
                format!("{index:02}_movie_pm_{}.png", name.trim_end_matches(".avi"))
            };
            let saved = save_frame(&out_dir, &file, &frame)?;
            let (audio_rate, audio_channels, audio_duration, audio_peak) = clip
                .as_ref()
                .map(|c| (c.sample_rate, c.channels, c.duration(), c.peak()))
                .unwrap_or((0, 0, 0.0, 0.0));
            if audio_peak <= 0.0 {
                return Err(format!("{edition} {name}: audio track is silent"));
            }
            entries.push(json!({
                "edition": edition,
                "name": name,
                "codec": avi.video.codec.name(),
                "width": avi.video.width,
                "height": avi.video.height,
                "fps": avi.video.fps,
                "frames": avi.video.frame_count,
                "duration": avi.duration(),
                "audio_codec": avi.audio.as_ref().map(|a| a.format.codec.name()),
                "audio_rate": audio_rate,
                "audio_channels": audio_channels,
                "audio_duration": audio_duration,
                "audio_peak": audio_peak,
                "lit_pixels": lit,
                "frame": saved,
            }));
            index += 1;
        }
    }
    if entries.is_empty() {
        return Err("mov.paz contains no decodable AVI entries".into());
    }

    // Prove the `.movie` VM instruction drives the real decoder and scene.
    let mut runtime = Runtime::open(&game, None).map_err(|e| e.to_string())?;
    runtime.set_auto_advance(true);
    runtime.start_script("memOp.sc").map_err(|e| e.to_string())?;
    let mut audio_written = 0;
    let mut out = vec![[0.0f32; 2]; 2048];
    for _ in 0..90 {
        runtime.tick_vm().map_err(|e| e.to_string())?;
        audio_written += runtime.mix_audio(&mut out);
    }
    if !runtime.movie_active() {
        let detail = runtime.last_error().unwrap_or("no error reported");
        let stats = runtime.stats();
        return Err(format!(
            "memOp.sc did not keep the movie active: {detail} (movies={}, errors={}, frame={})",
            stats.movies,
            stats.errors,
            runtime.frame_count()
        ));
    }
    runtime.render_now();
    let vm_entry = save_frame(&out_dir, "13_movie_vm_op.png", runtime.frame())?;
    let vm_lit = runtime
        .frame()
        .data
        .chunks_exact(4)
        .filter(|p| p[0] > 8 || p[1] > 8 || p[2] > 8)
        .count();
    if vm_lit < 500 {
        return Err(format!("VM movie frame is blank ({vm_lit} px)"));
    }
    if audio_written == 0 {
        return Err("VM movie audio was not mixed".into());
    }

    let report = json!({
        "generated_unix": now_unix(),
        "game_dir": game.display().to_string(),
        "time_seconds": time,
        "ffmpeg": true,
        "videos": entries,
        "vm_movie": {
            "script": "memOp.sc",
            "frame": vm_entry,
            "lit_pixels": vm_lit,
            "audio_frames_mixed": audio_written,
        },
        "ok": true,
    });
    write_json(&PathBuf::from("reports/movie_report.json"), &report)?;
    println!("movie: {} real AVI decodes", entries.len());
    for video in report["videos"].as_array().unwrap() {
        println!(
            "  {:<4} {:<7} {:<6} {}x{} @ {:.2}fps  lit={}  audio={} {}Hz {}ch",
            video["edition"].as_str().unwrap_or("?"),
            video["name"].as_str().unwrap_or("?"),
            video["codec"].as_str().unwrap_or("?"),
            video["width"],
            video["height"],
            video["fps"].as_f64().unwrap_or(0.0),
            video["lit_pixels"],
            video["audio_codec"].as_str().unwrap_or("?"),
            video["audio_rate"],
            video["audio_channels"],
        );
    }
    println!("  VM .movie frame lit={vm_lit}, audio frames mixed={audio_written}");
    println!("report: reports/movie_report.json");
    Ok(())
}

fn cmd_ui(args: &[String]) -> Result<(), String> {
    let opts = Opts::parse(args);
    let game = game_dir(&opts);
    let out_dir = PathBuf::from(opts.value("out").unwrap_or("reports/screenshots"));
    let max_frames = opts.number("frames", 30_000)?;
    let mut runtime = Runtime::open(&game, None).map_err(|e| e.to_string())?;
    runtime
        .set_save_dir(PathBuf::from("reports/saves"))
        .map_err(|e| e.to_string())?;
    runtime.use_system_font();
    runtime.set_auto_advance(true);

    runtime.open_ui(UiScreen::Title);
    let title = snap_ui(&mut runtime, &out_dir, "15_ui_title.png")?;

    runtime.start_script("A001.sc").map_err(|e| e.to_string())?;
    let mut reached = 0;
    for frame in 1..=max_frames {
        runtime.tick_vm().map_err(|e| e.to_string())?;
        if frame % 500 == 0 {
            runtime.refresh_ui_lists();
            let unlocked = runtime.ui().gallery.iter().filter(|g| g.unlocked).count();
            if runtime.ui().backlog.len() >= 20 && unlocked >= 3 {
                reached = frame;
                break;
            }
        }
    }
    runtime.refresh_ui_lists();
    if runtime.ui().backlog.len() < 5 {
        return Err(format!(
            "A001 produced only {} backlog entries",
            runtime.ui().backlog.len()
        ));
    }
    let unlocked_cgs = runtime.ui().gallery.iter().filter(|g| g.unlocked).count();
    if unlocked_cgs == 0 {
        return Err("A001 unlocked no CGs".into());
    }

    // Save the live stage into slot 0 and exercise the slot grid.
    runtime.save_slot(0).map_err(|e| e.to_string())?;
    runtime.open_ui(UiScreen::Save);
    runtime.ui_mut().clear_toast();
    let save_shot = snap_ui(&mut runtime, &out_dir, "16_ui_save.png")?;
    if runtime.ui_tap(600, 60) {
        // save slot 2 through the real hit region
        runtime.ui_mut().clear_toast();
        runtime.render_now();
    }
    runtime.open_ui(UiScreen::Load);
    runtime.ui_mut().clear_toast();
    let load_shot = snap_ui(&mut runtime, &out_dir, "17_ui_load.png")?;
    runtime.ui_tap(200, 60);
    if runtime.ui().screen != UiScreen::Game {
        return Err("load-slot tap did not return to the game".into());
    }
    runtime.ui_mut().cursor_visible = false;
    runtime.render_now();
    let game_after_load = save_frame(&out_dir, "17b_game_after_load.png", runtime.frame())?;

    runtime.open_ui(UiScreen::Backlog);
    runtime.ui_mut().clear_toast();
    let backlog_shot = snap_ui(&mut runtime, &out_dir, "18_ui_backlog.png")?;
    if runtime.ui_tap(40, 600) {
        // Back
    }

    runtime.open_ui(UiScreen::Gallery);
    runtime.ui_mut().clear_toast();
    let gallery_shot = snap_ui(&mut runtime, &out_dir, "19_ui_gallery.png")?;
    let thumb_count = runtime
        .ui()
        .gallery
        .iter()
        .filter(|g| g.thumb.is_some())
        .count();
    if thumb_count == 0 {
        return Err("gallery has no decoded cgthumb images".into());
    }
    let locked_cgs = runtime.ui().gallery.iter().filter(|g| !g.unlocked).count();
    if locked_cgs == 0 {
        return Err("gallery has no locked entries".into());
    }
    // Open the first unlocked CG fullscreen from the real hit region.
    let first_unlocked = runtime
        .ui()
        .gallery
        .iter()
        .position(|g| g.unlocked)
        .ok_or("gallery has no unlocked entries")?;
    let cell = first_unlocked % 25;
    let rect = (96 + (cell % 5) as i32 * 176, 30 + (cell / 5) as i32 * 113);
    runtime.ui_tap(rect.0 + 40, rect.1 + 30);
    runtime.ui_mut().clear_toast();
    let gallery_view_shot = snap_ui(&mut runtime, &out_dir, "19b_ui_gallery_view.png")?;
    if runtime.ui().screen != UiScreen::GalleryView {
        return Err("gallery tap did not open the fullscreen CG".into());
    }
    let full_pixels = runtime
        .ui()
        .gallery
        .get(first_unlocked)
        .and_then(|g| g.full.as_ref())
        .map(|f| f.data.chunks_exact(4).filter(|p| p[0] > 8 || p[1] > 8 || p[2] > 8).count())
        .unwrap_or(0);
    if full_pixels < 10_000 {
        return Err(format!("fullscreen CG is blank ({full_pixels} px)"));
    }
    runtime.ui_tap(500, 300);

    runtime.open_ui(UiScreen::Music);
    runtime.ui_mut().clear_toast();
    // Start the first unlocked track so the audit shot shows the native
    // playing highlight, note marker, level meter and progress bar.
    if let Some(index) = runtime.ui().music.iter().position(|m| m.unlocked) {
        for _ in 0..index / 16 {
            runtime.ui_tap(780, 594);
        }
        let row = index % 16;
        runtime.ui_tap(200, 42 + 32 * row as i32 + 6);
        for _ in 0..150 {
            runtime.ui_mut().tick();
        }
    }
    runtime.ui_mut().clear_toast();
    let music_shot = snap_ui(&mut runtime, &out_dir, "20_ui_music.png")?;
    let music_tracks = runtime.ui().music.len();
    if music_tracks == 0 {
        return Err("music room has no bgm.paz tracks".into());
    }
    let unlocked_tracks = runtime.ui().music.iter().filter(|m| m.unlocked).count();
    let locked_tracks = music_tracks - unlocked_tracks;
    if unlocked_tracks == 0 || locked_tracks == 0 {
        return Err(format!(
            "music room unlock linkage failed (unlocked={unlocked_tracks}, locked={locked_tracks})"
        ));
    }

    runtime.open_ui(UiScreen::Config);
    runtime.ui_tap(760, 115); // BGM slider, left half -> -5
    runtime.ui_mut().clear_toast();
    let config_shot = snap_ui(&mut runtime, &out_dir, "21_ui_config.png")?;

    runtime.open_ui(UiScreen::Extra);
    runtime.ui_mut().clear_toast();
    let extra_shot = snap_ui(&mut runtime, &out_dir, "22_ui_extra.png")?;

    let report = json!({
        "generated_unix": now_unix(),
        "game_dir": game.display().to_string(),
        "reached_frame": reached,
        "backlog_entries": runtime.ui().backlog.len(),
        "unlocked_cgs": unlocked_cgs,
        "locked_cgs": locked_cgs,
        "gallery_thumbnails": thumb_count,
        "gallery_full_pixels": full_pixels,
        "music_tracks": music_tracks,
        "unlocked_tracks": unlocked_tracks,
        "locked_tracks": locked_tracks,
        "screens": {
            "title": title,
            "save": save_shot,
            "load": load_shot,
            "game_after_load": game_after_load,
            "backlog": backlog_shot,
            "gallery": gallery_shot,
            "gallery_view": gallery_view_shot,
            "music": music_shot,
            "config": config_shot,
            "extra": extra_shot,
        },
        "ok": true,
    });
    write_json(&PathBuf::from("reports/ui_report.json"), &report)?;
    println!("ui: title/save/load/backlog/gallery/music/config/extra rendered");
    println!(
        "  A001 frame {reached}: backlog={} cgs={} thumbs={} tracks={}",
        runtime.ui().backlog.len(),
        unlocked_cgs,
        thumb_count,
        music_tracks
    );
    println!("report: reports/ui_report.json");
    Ok(())
}

fn cmd_display(args: &[String]) -> Result<(), String> {
    let opts = Opts::parse(args);
    let game = game_dir(&opts);
    let out_dir = PathBuf::from(opts.value("out").unwrap_or("reports/screenshots"));
    let mut runtime = Runtime::open(&game, None).map_err(|e| e.to_string())?;
    runtime.use_system_font();
    runtime.set_auto_advance(true);

    // 1) Scroll: stop at the ev006 vertical pan (stage y=1280, speed -7).
    runtime
        .start_script("A001.sc")
        .map_err(|e| e.to_string())?;
    let mut scroll_found = false;
    for _ in 0..20_000 {
        runtime.tick_vm().map_err(|e| e.to_string())?;
        let is_ev006 = runtime
            .vm()
            .and_then(|vm| vm.scroll())
            .map(|s| s.start_y == 1280 && s.speed_y == -7)
            .unwrap_or(false);
        if is_ev006 {
            scroll_found = true;
            break;
        }
    }
    if !scroll_found {
        return Err("A001 never reached the ev006 scroll".into());
    }
    runtime.reset_display_clock();
    runtime.render_now();
    let scroll_60_frame = runtime.frame().clone();
    let scroll_60 = save_frame(&out_dir, "23_display_60hz_scroll.png", &scroll_60_frame)?;
    let view_60 = runtime.scene().view_y;
    let ticks = runtime.tick_display(1.0 / 120.0).map_err(|e| e.to_string())?;
    if ticks != 0 {
        return Err(format!("half a 120 Hz frame consumed {ticks} logical ticks"));
    }
    runtime.render_now();
    let scroll_120 = save_frame(&out_dir, "24_display_120hz_scroll.png", runtime.frame())?;
    let sub_y = runtime.scene().view_sub_y;
    if (sub_y + 3.5).abs() > 1e-4 {
        return Err(format!("expected -3.5 px sub-pixel offset, got {sub_y}"));
    }
    let scroll_diff = pixel_diff(runtime.frame(), &scroll_60_frame);
    if scroll_diff < 10_000 {
        return Err(format!(
            "120 Hz scroll frame barely differs from 60 Hz ({scroll_diff} px)"
        ));
    }

    // 2) Transition: find a 60-frame wipe whose outgoing frame is real art
    // (A001 fades ev001 -> BLACK right after the first scene).
    runtime
        .start_script("A001.sc")
        .map_err(|e| e.to_string())?;
    let mut transition_found = false;
    for _ in 0..5000 {
        runtime.tick_vm().map_err(|e| e.to_string())?;
        let is_art_wipe = runtime
            .scene()
            .transition
            .as_ref()
            .map(|t| {
                t.total_frames == 60
                    && t.from
                        .as_ref()
                        .map(|f| f.pixel(0, 0) != [0, 0, 0, 255])
                        .unwrap_or(false)
            })
            .unwrap_or(false);
        if is_art_wipe {
            transition_found = true;
            break;
        }
    }
    if !transition_found {
        return Err("A001 never started a 60-frame art wipe".into());
    }
    // Let the new stage land and advance one logical frame into the wipe,
    // otherwise the "to" image is still identical to the captured "from".
    runtime.reset_display_clock();
    runtime.tick_display(1.0 / 60.0).map_err(|e| e.to_string())?;
    runtime.render_now();
    let trans_60_frame = runtime.frame().clone();
    let trans_60 = save_frame(&out_dir, "25_display_60hz_transition.png", &trans_60_frame)?;
    runtime.tick_display(1.0 / 120.0).map_err(|e| e.to_string())?;
    runtime.render_now();
    let trans_120 = save_frame(&out_dir, "26_display_120hz_transition.png", runtime.frame())?;
    let transition_sub = runtime.scene().transition_sub;
    if (transition_sub - 0.5).abs() > 1e-4 {
        return Err(format!(
            "expected 0.5 sub-frame transition progress, got {transition_sub}"
        ));
    }
    let trans_diff = pixel_diff(runtime.frame(), &trans_60_frame);
    if trans_diff < 10_000 {
        return Err(format!(
            "120 Hz transition frame barely differs from 60 Hz ({trans_diff} px)"
        ));
    }

    let report = json!({
        "ok": true,
        "scroll": {
            "view_y_60hz": view_60,
            "view_sub_y_120hz": sub_y,
            "changed_pixels": scroll_diff,
            "frame_60hz": scroll_60,
            "frame_120hz": scroll_120,
        },
        "transition": {
            "sub_frame_120hz": transition_sub,
            "changed_pixels": trans_diff,
            "frame_60hz": trans_60,
            "frame_120hz": trans_120,
        },
    });
    write_json(&PathBuf::from("reports/display_report.json"), &report)?;
    println!(
        "display: 120 Hz interpolation ok (scroll sub {sub_y:.1} px, {scroll_diff} px changed; \
         transition sub {transition_sub:.2}, {trans_diff} px changed)"
    );
    println!("report: reports/display_report.json");
    Ok(())
}

fn pixel_diff(a: &RgbaImage, b: &RgbaImage) -> usize {
    a.data
        .chunks_exact(4)
        .zip(b.data.chunks_exact(4))
        .filter(|(x, y)| x != y)
        .count()
}

fn cmd_walkthrough(args: &[String]) -> Result<(), String> {
    let opts = Opts::parse(args);
    let game = game_dir(&opts);
    let out = PathBuf::from(
        opts.value("json")
            .unwrap_or("reports/full_walkthrough_report.json"),
    );
    let max_frames = opts.number("max-frames", 2_000_000)?;
    let which = opts.value("edition").unwrap_or("all");
    let mut editions: Vec<(&str, PathBuf, Option<PathBuf>)> = Vec::new();
    if which == "all" || which == "main" {
        editions.push(("main", game.clone(), None));
    }
    if which == "all" || which == "pm" {
        let pm = pm_dir(&game);
        if pm != game {
            let exe = ["eden_pm_en.exe", "eden_pm.exe"]
                .iter()
                .map(|n| pm.join(n))
                .find(|p| p.exists());
            editions.push(("pm", pm, exe));
        }
    }
    if editions.is_empty() {
        return Err("no editions selected".into());
    }

    let mut reports = Vec::new();
    let mut total_scripts = 0;
    let mut total_ended = 0;
    let mut total_errors = 0;
    let started = std::time::Instant::now();
    for (edition, dir, exe) in &editions {
        let mut runtime = Runtime::open(dir, exe.as_deref())
            .map_err(|e| format!("{}: {e}", dir.display()))?;
        runtime.set_headless(true);
        runtime.set_video_enabled(false);
        runtime.set_auto_advance(true);
        let names = runtime.script_names();
        println!("walkthrough {edition}: {} scripts", names.len());
        for name in names {
            total_scripts += 1;
            runtime.reset_stats();
            let mut frames = 0u64;
            let mut state = String::new();
            let mut error = None;
            let mut pc = 0;
            let mut commands = 0;
            match runtime.start_script(&name) {
                Ok(()) => {
                    commands = runtime.vm().map(|v| v.script().commands.len()).unwrap_or(0);
                    loop {
                        if frames >= max_frames as u64 {
                            error = Some(format!("timed out after {max_frames} frames"));
                            break;
                        }
                        if let Err(e) = runtime.tick_vm() {
                            error = Some(e.to_string());
                            break;
                        }
                        frames += 1;
                        let vm = runtime.vm().unwrap();
                        state = format!("{:?}", vm.state());
                        pc = vm.pc();
                        if matches!(vm.state(), VmState::Ended) {
                            break;
                        }
                        if let Some(err) = runtime.last_error() {
                            error = Some(err.to_string());
                            break;
                        }
                    }
                }
                Err(e) => error = Some(e.to_string()),
            }
            let ended = state.starts_with("Ended");
            if ended {
                total_ended += 1;
            }
            if error.is_some() {
                total_errors += 1;
            }
            let stats = runtime.stats().clone();
            if error.is_none() && !ended {
                error = Some(format!("did not end (state={state})"));
                total_errors += 1;
            }
            let stage = runtime
                .vm()
                .map(|vm| vm.stage().image.clone())
                .unwrap_or_default();
            let backlog = runtime.ui().backlog.len();
            reports.push(json!({
                "edition": edition,
                "script": name,
                "commands": commands,
                "frames": frames,
                "ended": ended,
                "state": state,
                "pc": pc,
                "stage": stage,
                "backlog": backlog,
                "stats": {
                    "events": stats.events,
                    "messages": stats.messages,
                    "stages": stats.stages,
                    "transitions": stats.transitions,
                    "panels": stats.panels,
                    "movies": stats.movies,
                    "chains": stats.chains,
                    "scrolls": stats.scrolls,
                    "errors": stats.errors,
                },
                "error": error,
            }));
        }
    }
    let elapsed = started.elapsed().as_secs_f64();
    let total_frames: u64 = reports
        .iter()
        .map(|r| r["frames"].as_u64().unwrap_or(0))
        .sum();
    let total_messages: u64 = reports
        .iter()
        .map(|r| r["stats"]["messages"].as_u64().unwrap_or(0))
        .sum();
    let total_stages: u64 = reports
        .iter()
        .map(|r| r["stats"]["stages"].as_u64().unwrap_or(0))
        .sum();
    let total_transitions: u64 = reports
        .iter()
        .map(|r| r["stats"]["transitions"].as_u64().unwrap_or(0))
        .sum();
    let total_scrolls: u64 = reports
        .iter()
        .map(|r| r["stats"]["scrolls"].as_u64().unwrap_or(0))
        .sum();
    let total_panels: u64 = reports
        .iter()
        .map(|r| r["stats"]["panels"].as_u64().unwrap_or(0))
        .sum();
    let total_movies: u64 = reports
        .iter()
        .map(|r| r["stats"]["movies"].as_u64().unwrap_or(0))
        .sum();
    let total_chains: u64 = reports
        .iter()
        .map(|r| r["stats"]["chains"].as_u64().unwrap_or(0))
        .sum();
    let report = json!({
        "generated_unix": now_unix(),
        "game_dir": game.display().to_string(),
        "max_frames_per_script": max_frames,
        "editions": editions.iter().map(|(e, d, _)| json!({"edition": e, "dir": d.display().to_string()})).collect::<Vec<_>>(),
        "summary": {
            "scripts": total_scripts,
            "ended": total_ended,
            "errors": total_errors,
            "elapsed_seconds": elapsed,
            "frames": total_frames,
            "messages": total_messages,
            "stages": total_stages,
            "transitions": total_transitions,
            "scrolls": total_scrolls,
            "panels": total_panels,
            "movies": total_movies,
            "chains": total_chains,
        },
        "scripts": reports,
        "ok": total_errors == 0 && total_scripts == 58 && total_ended == 58,
    });
    write_json(&out, &report)?;
    println!(
        "walkthrough: {total_scripts} scripts, {total_ended} ended, {total_errors} errors in {elapsed:.2}s"
    );
    println!("report: {}", out.display());
    if total_errors != 0 || total_scripts != 58 || total_ended != 58 {
        return Err(format!(
            "walkthrough incomplete: {total_scripts} scripts, {total_ended} ended, {total_errors} errors"
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// list / audio
// ---------------------------------------------------------------------------

fn cmd_list(args: &[String]) -> Result<(), String> {
    let opts = Opts::parse(args);
    let path = opts
        .positional
        .first()
        .ok_or("usage: eden list <archive.paz> [--filter TEXT]")?;
    let filter = opts.value("filter").unwrap_or("").to_ascii_lowercase();
    let archive = PazArchive::open(path).map_err(|e| format!("{path}: {e}"))?;
    println!(
        "{}: id={} edition={:?} slot={} entries={}",
        path,
        archive.ident(),
        archive.edition(),
        archive.slot(),
        archive.len()
    );
    for entry in archive.entries() {
        if filter.is_empty() || entry.name.to_ascii_lowercase().contains(&filter) {
            println!(
                "{}\t{}\t{}\t{}",
                entry.name, entry.offset, entry.unpacked_size, entry.size
            );
        }
    }
    Ok(())
}

/// Export a complete catalogue of `sys.paz`: names, dimensions and `.ani`
/// frame metadata. Optionally extracts every decoded image for visual review.
fn cmd_catalog(args: &[String]) -> Result<(), String> {
    let opts = Opts::parse(args);
    let game = game_dir(&opts);
    let path = game.join("sys.paz");
    let archive = PazArchive::open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let extract = opts.value("extract").map(PathBuf::from);
    if let Some(dir) = &extract {
        fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let mut files = Vec::new();
    let mut kinds = BTreeMap::<String, usize>::new();
    for entry in archive.entries() {
        let ext = entry.extension();
        *kinds.entry(ext.clone()).or_default() += 1;
        let mut item = json!({
            "name": entry.name,
            "ext": ext,
            "bytes": entry.unpacked_size,
        });
        match ext.as_str() {
            "png" => match archive.decode_png(&entry.name) {
                Ok(img) => {
                    item["width"] = json!(img.width);
                    item["height"] = json!(img.height);
                    if let Some(dir) = &extract {
                        let rel = entry.name.replace('\\', "/");
                        let out = dir.join(rel);
                        if let Some(parent) = out.parent() {
                            fs::create_dir_all(parent)
                                .map_err(|e| format!("{}: {e}", parent.display()))?;
                        }
                        fs::write(&out, encode_png(&img).map_err(|e| e.to_string())?)
                            .map_err(|e| format!("{}: {e}", out.display()))?;
                    }
                }
                Err(e) => item["error"] = json!(e.to_string()),
            },
            "ani" => match archive.parse_ani(&entry.name) {
                Ok(ani) => {
                    let frames: Vec<Value> = ani
                        .frames
                        .iter()
                        .map(|f| {
                            json!({
                                "label": f.label,
                                "width": f.width,
                                "height": f.height,
                                "bpp": f.bpp,
                                "origin_x": f.origin_x,
                                "origin_y": f.origin_y,
                            })
                        })
                        .collect();
                    item["version"] = json!(ani.version);
                    item["frame_count"] = json!(ani.frames.len());
                    item["frames"] = json!(frames);
                    if let Some(dir) = &extract {
                        let rel = entry.name.replace('\\', "/");
                        let out = dir.join(format!("{rel}.json"));
                        if let Some(parent) = out.parent() {
                            fs::create_dir_all(parent)
                                .map_err(|e| format!("{}: {e}", parent.display()))?;
                        }
                        fs::write(&out, serde_json::to_vec_pretty(&frames).unwrap())
                            .map_err(|e| format!("{}: {e}", out.display()))?;
                    }
                }
                Err(e) => item["error"] = json!(e.to_string()),
            },
            _ => {}
        }
        files.push(item);
    }
    let report = json!({
        "archive": path.display().to_string(),
        "ident": archive.ident(),
        "edition": format!("{:?}", archive.edition()),
        "entry_count": archive.len(),
        "kinds": kinds,
        "files": files,
    });
    let out = PathBuf::from(opts.value("out").unwrap_or("reports/sys_paz_catalog.json"));
    write_json(&out, &report)?;
    println!(
        "catalog: {} entries -> {} ({} kinds)",
        archive.len(),
        out.display(),
        report["kinds"].as_object().map(|m| m.len()).unwrap_or(0)
    );
    if let Some(dir) = extract {
        println!("extracted to {}", dir.display());
    }
    Ok(())
}

fn cmd_audio(args: &[String]) -> Result<(), String> {
    let opts = Opts::parse(args);
    let game = game_dir(&opts);
    let voice = opts.value("voice").unwrap_or("sio-C013-0183");
    let archive = PazArchive::open(game.join("voice.paz"))
        .map_err(|e| format!("{}: {e}", game.join("voice.paz").display()))?;
    let clip =
        eden_audio::decode_paz_entry(&archive, voice).map_err(|e| format!("{voice}: {e}"))?;
    let analyzer = eden_audio::LipSyncAnalyzer::new(4);
    let mut histogram = [0usize; 4];
    for block in clip.samples.chunks(1024) {
        histogram[analyzer.process(block) as usize] += 1;
    }
    let report = json!({
        "voice": voice,
        "sample_rate": clip.sample_rate,
        "channels": clip.channels,
        "frames": clip.frames(),
        "duration": clip.duration(),
        "peak": clip.peak(),
        "rms": clip.rms(),
        "lipsync_frames": histogram,
        "envelope": analyzer.envelope(),
    });
    let report_path = PathBuf::from(opts.value("json").unwrap_or("reports/audio_voice.json"));
    write_json(&report_path, &report)?;
    println!(
        "audio: {voice} {} Hz x{} {:.3}s peak={:.4} rms={:.4} lipsync={:?}",
        clip.sample_rate,
        clip.channels,
        clip.duration(),
        clip.peak(),
        clip.rms(),
        histogram
    );
    println!("report: {}", report_path.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_options_separates_flags_and_values() {
        let args = vec![
            "--game".to_string(),
            "/tmp/eden".to_string(),
            "--frames=120".to_string(),
            "--pm".to_string(),
            "A001.sc".to_string(),
        ];
        let opts = Opts::parse(&args);
        assert_eq!(opts.value("game"), Some("/tmp/eden"));
        assert_eq!(opts.value("frames"), Some("120"));
        assert!(opts.flag("pm"));
        assert_eq!(opts.positional, vec!["A001.sc"]);
    }

    #[test]
    fn event_names_are_stable() {
        assert_eq!(event_name(&VmEvent::TransitionEnd), "transitionEnd");
        assert_eq!(
            event_name(&VmEvent::Error {
                message: "x".into(),
                line: 1
            }),
            "error"
        );
    }

    #[test]
    fn sniff_rejects_wrong_magic() {
        assert!(sniff("png", b"\x89PNG\r\n\x1a\n").is_ok());
        assert!(sniff("png", b"OggS").is_err());
        assert!(sniff("ogg", b"OggS").is_ok());
        assert!(sniff("wav", b"RIFF\0\0\0\0WAVE").is_ok());
        assert!(sniff("avi", b"RIFF\0\0\0\0AVI ").is_ok());
    }
}
