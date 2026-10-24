//! Line-oriented `.sc` parser.

use std::collections::HashMap;

use crate::command::*;
use crate::encoding::{decode_script, ScriptEncoding};
use crate::error::{ParseError, ParseWarning};
use crate::text::MessageText;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScriptStats {
    pub physical_lines: usize,
    pub blank_lines: usize,
    pub comment_lines: usize,
    pub code_lines: usize,
}

#[derive(Clone, Debug)]
pub struct Script {
    pub name: String,
    pub commands: Vec<ScriptCommand>,
    pub labels: HashMap<String, usize>,
    pub stats: ScriptStats,
    pub errors: Vec<ParseError>,
    pub warnings: Vec<ParseWarning>,
}

impl Script {
    pub fn is_valid(&self) -> bool {
        self.errors.is_empty()
    }

    pub fn label(&self, name: &str) -> Option<usize> {
        self.labels.get(name).copied()
    }

    pub fn command_count(&self, name: &str) -> usize {
        self.commands.iter().filter(|c| c.name() == name).count()
    }
}

/// Parse a UTF-8 script that has already been decoded.
pub fn parse_script(name: impl Into<String>, text: &str) -> Script {
    let name = name.into();
    // Normalise CRLF and lone CR. `.lines()` then gives physical lines without
    // a phantom trailing empty line.
    let normalised = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut commands = Vec::new();
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    let mut stats = ScriptStats::default();
    let mut labels: HashMap<String, usize> = HashMap::new();

    for (idx, raw_line) in normalised.lines().enumerate() {
        let line_no = idx + 1;
        stats.physical_lines += 1;
        let line = raw_line.trim();
        if line.is_empty() {
            stats.blank_lines += 1;
            continue;
        }
        if line.starts_with(';') {
            stats.comment_lines += 1;
            continue;
        }
        stats.code_lines += 1;
        match parse_line(line, line_no) {
            Ok(cmd) => {
                if let ScriptCommand::Label(l) = &cmd {
                    if let Some(prev) = labels.insert(l.name.clone(), commands.len()) {
                        warnings.push(ParseWarning {
                            line: line_no,
                            source: line.to_string(),
                            message: format!(
                                "duplicate label {:?} (previous command index {prev})",
                                l.name
                            ),
                        });
                    }
                }
                commands.push(cmd);
            }
            Err(message) => errors.push(ParseError {
                line: line_no,
                source: line.to_string(),
                message,
            }),
        }
    }

    Script {
        name,
        commands,
        labels,
        stats,
        errors,
        warnings,
    }
}

/// Decode raw archive bytes and parse them.
pub fn parse_script_bytes(
    name: impl Into<String>,
    bytes: &[u8],
    encoding: ScriptEncoding,
) -> Result<Script, crate::error::ScriptError> {
    let text = decode_script(bytes, encoding)?;
    Ok(parse_script(name, &text))
}

fn parse_line(line: &str, line_no: usize) -> Result<ScriptCommand, String> {
    let (cmd, rest) = if let Some(after) = line.strip_prefix('.') {
        let end = after.find(char::is_whitespace).unwrap_or(after.len());
        (format!(".{}", &after[..end]), after[end..].trim_start())
    } else if line.starts_with('#') {
        let end = line.find(char::is_whitespace).unwrap_or(line.len());
        (line[..end].to_string(), line[end..].trim_start())
    } else {
        return Err("line does not start with '.' or '#'".into());
    };
    let args: Vec<&str> = rest.split_whitespace().collect();
    let cmd_ref = cmd.as_str();

    match cmd_ref {
        ".message" => parse_message(rest, line_no),
        ".stage" => {
            expect_args(&args, 4, cmd_ref)?;
            Ok(ScriptCommand::Stage(StageCommand {
                mask: star_or(&args[0]),
                image: args[1].to_string(),
                x: int(&args[2], cmd_ref)?,
                y: int(&args[3], cmd_ref)?,
                line: line_no,
            }))
        }
        ".transition" => {
            expect_args(&args, 3, cmd_ref)?;
            Ok(ScriptCommand::Transition(TransitionCommand {
                kind: int(&args[0], cmd_ref)?,
                layer: star_or(&args[1]),
                frames: uint(&args[2], cmd_ref)?,
                line: line_no,
            }))
        }
        ".panel" => {
            if args.is_empty() || args.len() > 3 {
                return Err(format!(".panel expects 1..3 arguments, got {}", args.len()));
            }
            let mode = int(args[0], cmd_ref)?;
            // Syntax: `.panel <mode> [<mask> [<image>]]`. The mask argument is
            // `*` when unused; the image argument is `-` when unused.
            let mask = if args.len() >= 2 {
                star_or(&args[1])
            } else {
                None
            };
            let image = if args.len() == 3 && args[2] != "-" {
                Some(args[2].to_string())
            } else {
                None
            };
            Ok(ScriptCommand::Panel(PanelCommand {
                mode,
                mask,
                image,
                line: line_no,
            }))
        }
        ".playBGM" => parse_play_bgm(&args, line_no),
        ".playSE" => parse_play_se(&args, line_no, false),
        ".playSE2" => parse_play_se(&args, line_no, true),
        ".playVoice" => parse_play_voice(&args, line_no),
        ".wait" => {
            expect_args(&args, 1, cmd_ref)?;
            Ok(ScriptCommand::Wait(WaitCommand {
                frames: uint(args[0], cmd_ref)?,
                line: line_no,
            }))
        }
        ".pragma" => {
            expect_args(&args, 1, cmd_ref)?;
            let disable_control = match args[0] {
                "disable_control" => true,
                "enable_control" => false,
                other => return Err(format!(".pragma unknown argument {other:?}")),
            };
            Ok(ScriptCommand::Pragma(PragmaCommand {
                disable_control,
                line: line_no,
            }))
        }
        ".label" => {
            expect_args(&args, 1, cmd_ref)?;
            Ok(ScriptCommand::Label(LabelCommand {
                name: args[0].to_string(),
                line: line_no,
            }))
        }
        ".if" => {
            expect_args(&args, 4, cmd_ref)?;
            let op = CompareOp::parse(args[1])
                .ok_or_else(|| format!(".if unknown operator {:?}", args[1]))?;
            Ok(ScriptCommand::If(IfCommand {
                var: args[0].to_string(),
                op,
                value: int64(args[2], cmd_ref)?,
                label: args[3].to_string(),
                line: line_no,
            }))
        }
        ".endScroll" => {
            expect_args(&args, 1, cmd_ref)?;
            Ok(ScriptCommand::EndScroll(EndScrollCommand {
                flag: tf(args[0], cmd_ref)?,
                line: line_no,
            }))
        }
        ".chain" => {
            expect_args(&args, 1, cmd_ref)?;
            Ok(ScriptCommand::Chain(ChainCommand {
                file: args[0].to_string(),
                line: line_no,
            }))
        }
        ".setGlobal" | ".set" => {
            let (var, value) = parse_assignment(&args, cmd_ref)?;
            let c = SetVarCommand {
                var,
                value,
                line: line_no,
            };
            if cmd_ref == ".setGlobal" {
                Ok(ScriptCommand::SetGlobal(c))
            } else {
                Ok(ScriptCommand::Set(c))
            }
        }
        ".vscroll" => {
            expect_args(&args, 2, cmd_ref)?;
            Ok(ScriptCommand::VScroll(VScrollCommand {
                target: int(args[0], cmd_ref)?,
                speed: int(args[1], cmd_ref)?,
                line: line_no,
            }))
        }
        ".hscroll" => {
            expect_args(&args, 2, cmd_ref)?;
            Ok(ScriptCommand::HScroll(HScrollCommand {
                target: int(args[0], cmd_ref)?,
                speed: int(args[1], cmd_ref)?,
                line: line_no,
            }))
        }
        ".scroll" => {
            expect_args(&args, 3, cmd_ref)?;
            Ok(ScriptCommand::Scroll(ScrollCommand {
                target_x: int(args[0], cmd_ref)?,
                target_y: int(args[1], cmd_ref)?,
                frames: uint(args[2], cmd_ref)?,
                line: line_no,
            }))
        }
        ".movie" => {
            expect_args(&args, 5, cmd_ref)?;
            Ok(ScriptCommand::Movie(MovieCommand {
                id: int64(args[0], cmd_ref)?,
                file: args[1].to_string(),
                width: uint(args[2], cmd_ref)?,
                height: uint(args[3], cmd_ref)?,
                wait: tf(args[4], cmd_ref)?,
                line: line_no,
            }))
        }
        ".goto" => {
            expect_args(&args, 1, cmd_ref)?;
            Ok(ScriptCommand::Goto(GotoCommand {
                label: args[0].to_string(),
                line: line_no,
            }))
        }
        ".shakeScreen" => {
            expect_args(&args, 3, cmd_ref)?;
            let axis = match args[0] {
                "R" => ShakeAxis::Random,
                "Z" => ShakeAxis::Z,
                other => return Err(format!(".shakeScreen unknown axis {other:?}")),
            };
            Ok(ScriptCommand::ShakeScreen(ShakeCommand {
                axis,
                amplitude: int(args[1], cmd_ref)?,
                frames: uint(args[2], cmd_ref)?,
                line: line_no,
            }))
        }
        ".effect" => {
            if args.len() == 1 && args[0].eq_ignore_ascii_case("end") {
                return Ok(ScriptCommand::Effect(EffectCommand::End { line: line_no }));
            }
            if args.len() == 4 {
                let name = args[0].to_string();
                let count = uint(args[2], cmd_ref)?;
                let frames = uint(args[3], cmd_ref)?;
                if name.eq_ignore_ascii_case("Firefly") {
                    return Ok(ScriptCommand::Effect(EffectCommand::Firefly {
                        count,
                        frames,
                        line: line_no,
                    }));
                }
                return Ok(ScriptCommand::Effect(EffectCommand::Other {
                    name,
                    args: args.iter().map(|s| s.to_string()).collect(),
                    line: line_no,
                }));
            }
            Ok(ScriptCommand::Effect(EffectCommand::Other {
                name: args.first().copied().unwrap_or("").to_string(),
                args: args.iter().map(|s| s.to_string()).collect(),
                line: line_no,
            }))
        }
        ".end" => Ok(ScriptCommand::End { line: line_no }),
        _ => {
            let known = is_compat_command(cmd_ref);
            Ok(ScriptCommand::Compat(CompatCommand {
                name: cmd,
                args: args.iter().map(|s| s.to_string()).collect(),
                known,
                line: line_no,
            }))
        }
    }
}

fn parse_message(rest: &str, line_no: usize) -> Result<ScriptCommand, String> {
    let tokens = tokenize(rest);
    if tokens.is_empty() {
        return Err(".message expects an id".into());
    }
    let id = int64(tokens[0].1, ".message")?;
    let mut idx = 1;
    let voice = if idx < tokens.len() && is_voice_token(tokens[idx].1) {
        let v = tokens[idx].1.to_string();
        idx += 1;
        Some(v)
    } else {
        None
    };

    // Voiced lines always carry a speaker token: `.message 100 <voice> 亮\n...`
    // in the main scripts, `.message 120 <voice> Maya “...”` in PLUS+MOSAIC,
    // and `.message 100 <voice> #诗音 某一日...` when the name is marked with
    // `#`. Narration lines have no voice and therefore no speaker.
    let mut speaker = None;
    let mut text_start = if idx < tokens.len() {
        tokens[idx].0
    } else {
        rest.len()
    };
    if voice.is_some() {
        let mut pos = text_start;
        if rest[pos..].starts_with('#') {
            pos += 1;
        }
        let tail = &rest[pos..];
        let name_end = tail
            .find(|c: char| c.is_whitespace() || c == '\\')
            .unwrap_or(tail.len());
        let name = &tail[..name_end];
        let name_ok = !name.is_empty()
            && name.chars().count() <= 16
            && !name.contains(['“', '”', '"', '乬', '乭', '#']);
        if name_ok {
            speaker = Some(name.to_string());
            pos += name_end;
            while let Some(c) = rest[pos..].chars().next() {
                if c.is_whitespace() {
                    pos += c.len_utf8();
                } else {
                    break;
                }
            }
            // The name/body separator is a hard line break in the source.
            if rest[pos..].starts_with("\\n") {
                pos += 2;
            }
            text_start = pos;
        }
    } else if rest[text_start..].starts_with('#') {
        // Narration never uses `#`, but keep the historical behaviour safe.
        text_start += 1;
    }

    let raw_text = rest[text_start..].trim_start().to_string();
    Ok(ScriptCommand::Message(MessageCommand {
        id,
        voice,
        speaker,
        text: MessageText::parse(raw_text),
        line: line_no,
    }))
}

fn parse_play_bgm(args: &[&str], line_no: usize) -> Result<ScriptCommand, String> {
    if args.is_empty() {
        return Err(".playBGM expects at least one argument".into());
    }
    let file = star_or(args[0]);
    let mut volume = None;
    let mut fade_frames = None;
    if args.len() >= 2 && args[1] != "*" {
        volume = Some(int(args[1], ".playBGM")?);
    }
    if args.len() >= 3 && args[2] != "*" {
        fade_frames = Some(int(args[2], ".playBGM")?);
    } else if args.len() == 2 && args[1] != "*" && file.is_none() {
        // `* <frames>` is not used by eden*, but keep it safe.
        fade_frames = Some(int(args[1], ".playBGM")?);
    }
    Ok(ScriptCommand::PlayBgm(PlayBgmCommand {
        file,
        volume,
        fade_frames,
        raw_args: args.iter().map(|s| s.to_string()).collect(),
        line: line_no,
    }))
}

fn parse_play_se(args: &[&str], line_no: usize, second: bool) -> Result<ScriptCommand, String> {
    let cmd = if second { ".playSE2" } else { ".playSE" };
    if args.is_empty() {
        return Err(format!("{cmd} expects at least one argument"));
    }
    let mut file = None;
    let mut volume = None;
    let mut volume_pair = None;
    let mut idx = 0;
    if args[0] != "*" {
        let (name, vol, pair) = split_volume(args[0])?;
        file = Some(name);
        volume = vol;
        volume_pair = pair;
    }
    idx += 1;
    let mut looping = false;
    if idx < args.len() && matches!(args[idx], "t" | "f" | "true" | "false") {
        looping = matches!(args[idx], "t" | "true");
        idx += 1;
    }
    let mut pan = 0.0f32;
    if idx < args.len() && args[idx] != "*" {
        // The retail corpus contains a typo (`.playSE ... true O *`); the
        // engine's atoi-style parser treats it as 0. Do the same instead of
        // rejecting the script.
        pan = args[idx].parse::<f32>().unwrap_or(0.0);
        idx += 1;
    }
    let reserved = if idx < args.len() {
        Some(args[idx].to_string())
    } else {
        None
    };
    let c = PlaySeCommand {
        file,
        volume,
        volume_pair,
        looping,
        pan,
        reserved,
        raw_args: args.iter().map(|s| s.to_string()).collect(),
        line: line_no,
    };
    Ok(if second {
        ScriptCommand::PlaySe2(c)
    } else {
        ScriptCommand::PlaySe(c)
    })
}

fn parse_play_voice(args: &[&str], line_no: usize) -> Result<ScriptCommand, String> {
    if args.is_empty() {
        return Err(".playVoice expects at least one argument".into());
    }
    let file = star_or(args[0]);
    let mut volume = None;
    for a in &args[1..] {
        if *a != "*" {
            if let Ok(v) = a.parse::<i32>() {
                volume = Some(v);
            }
        }
    }
    Ok(ScriptCommand::PlayVoice(PlayVoiceCommand {
        file,
        volume,
        raw_args: args.iter().map(|s| s.to_string()).collect(),
        line: line_no,
    }))
}

fn split_volume(token: &str) -> Result<(String, Option<f32>, Option<(i32, i32)>), String> {
    if let Some(open) = token.find('[') {
        let close = token
            .rfind(']')
            .ok_or_else(|| format!("unterminated volume in {token:?}"))?;
        if close <= open {
            return Err(format!("invalid volume in {token:?}"));
        }
        let name = token[..open].to_string();
        let inside = &token[open + 1..close];
        let vals: Vec<i32> = inside
            .split(',')
            .map(|s| s.trim().parse::<i32>())
            .collect::<Result<_, _>>()
            .map_err(|_| format!("invalid volume in {token:?}"))?;
        if vals.is_empty() {
            return Err(format!("empty volume in {token:?}"));
        }
        let pair = if vals.len() >= 2 {
            Some((vals[0], vals[1]))
        } else {
            None
        };
        Ok((name, Some(vals[0] as f32 / 100.0), pair))
    } else {
        Ok((token.to_string(), None, None))
    }
}

fn parse_assignment(args: &[&str], cmd: &str) -> Result<(String, i64), String> {
    if args.len() < 3 {
        return Err(format!("{cmd} expects `<var> = <value>`"));
    }
    if args[1] != "=" {
        return Err(format!("{cmd} expects '=' after variable name"));
    }
    Ok((args[0].to_string(), int64(args[2], cmd)?))
}

fn tokenize(s: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    let mut start = None;
    for (i, ch) in s.char_indices() {
        if ch.is_whitespace() {
            if let Some(st) = start.take() {
                out.push((st, &s[st..i]));
            }
        } else if start.is_none() {
            start = Some(i);
        }
    }
    if let Some(st) = start {
        out.push((st, &s[st..]));
    }
    out
}

fn is_voice_token(s: &str) -> bool {
    if s == "*" {
        return true;
    }
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() != 3 {
        return false;
    }
    let (a, b, c) = (parts[0], parts[1], parts[2]);
    !a.is_empty()
        && a.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
        && b.len() == 4
        && b.as_bytes()[0].is_ascii_alphabetic()
        && b[1..].chars().all(|ch| ch.is_ascii_digit())
        && c.len() >= 4
        && c[..4].chars().all(|ch| ch.is_ascii_digit())
}

fn star_or(s: &str) -> Option<String> {
    if s == "*" {
        None
    } else {
        Some(s.to_string())
    }
}

fn expect_args(args: &[&str], n: usize, cmd: &str) -> Result<(), String> {
    if args.len() != n {
        return Err(format!("{cmd} expects {n} arguments, got {}", args.len()));
    }
    Ok(())
}

fn int(s: &str, cmd: &str) -> Result<i32, String> {
    s.parse::<i32>()
        .map_err(|_| format!("{cmd}: invalid integer {s:?}"))
}

fn int64(s: &str, cmd: &str) -> Result<i64, String> {
    s.parse::<i64>()
        .map_err(|_| format!("{cmd}: invalid integer {s:?}"))
}

fn uint(s: &str, cmd: &str) -> Result<u32, String> {
    s.parse::<u32>()
        .map_err(|_| format!("{cmd}: invalid unsigned integer {s:?}"))
}

fn tf(s: &str, cmd: &str) -> Result<bool, String> {
    match s {
        "t" | "true" => Ok(true),
        "f" | "false" => Ok(false),
        other => Err(format!("{cmd}: expected t/f, got {other:?}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_panel_mask_and_image() {
        let s = parse_script(
            "t.sc",
            ".panel 1 * msgPanel_Omni.png\n.panel 2 * -\n.panel 0\n",
        );
        assert!(s.errors.is_empty(), "{:?}", s.errors);
        match &s.commands[0] {
            ScriptCommand::Panel(p) => {
                assert_eq!(p.mode, 1);
                assert_eq!(p.mask, None);
                assert_eq!(p.image.as_deref(), Some("msgPanel_Omni.png"));
            }
            other => panic!("unexpected {other:?}"),
        }
        match &s.commands[1] {
            ScriptCommand::Panel(p) => {
                assert_eq!(p.mode, 2);
                assert_eq!(p.mask, None);
                assert_eq!(p.image, None);
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn parses_message_with_voice_and_hash() {
        let s = parse_script(
            "t.sc",
            ".message 100 ryo-A001-1003 #亮 “我不能停下双手”\\v\\a\n",
        );
        assert!(s.errors.is_empty(), "{:?}", s.errors);
        match &s.commands[0] {
            ScriptCommand::Message(m) => {
                assert_eq!(m.id, 100);
                assert_eq!(m.voice.as_deref(), Some("ryo-A001-1003"));
                assert_eq!(m.speaker.as_deref(), Some("亮"));
                assert_eq!(m.text.plain_text(), "“我不能停下双手”");
                assert!(m.text.has_wait_click());
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn parses_message_without_voice_or_hash() {
        let s = parse_script("t.sc", ".message 180   醒来时，耳旁感到些许违和。\n");
        assert!(s.errors.is_empty(), "{:?}", s.errors);
        match &s.commands[0] {
            ScriptCommand::Message(m) => {
                assert_eq!(m.voice, None);
                assert_eq!(m.text.plain_text(), "醒来时，耳旁感到些许违和。");
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn parses_message_voice_without_hash() {
        let s = parse_script("t.sc", ".message 210 ryo-C001-0003 亮\\n“算了……”\n");
        match &s.commands[0] {
            ScriptCommand::Message(m) => {
                assert_eq!(m.voice.as_deref(), Some("ryo-C001-0003"));
                assert_eq!(m.speaker.as_deref(), Some("亮"));
                assert_eq!(m.text.plain_text(), "“算了……”");
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn parses_plus_mosaic_speaker_and_inline_codes() {
        let s = parse_script(
            "t.sc",
            ".message 1580 may-H004-0105 Maya “Okay#4... squeezing#5 tight.”\n",
        );
        assert!(s.errors.is_empty(), "{:?}", s.errors);
        match &s.commands[0] {
            ScriptCommand::Message(m) => {
                assert_eq!(m.voice.as_deref(), Some("may-H004-0105"));
                assert_eq!(m.speaker.as_deref(), Some("Maya"));
                assert_eq!(m.text.plain_text(), "“Okay... squeezing tight.”");
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn parses_playse_volume_pair() {
        let s = parse_script("t.sc", ".playSE hiro_reizouko_open.ogg[100,100] f 0 *\n");
        match &s.commands[0] {
            ScriptCommand::PlaySe(p) => {
                assert_eq!(p.file.as_deref(), Some("hiro_reizouko_open.ogg"));
                assert_eq!(p.volume, Some(1.0));
                assert_eq!(p.volume_pair, Some((100, 100)));
                assert!(!p.looping);
                assert_eq!(p.pan, 0.0);
                assert_eq!(p.reserved.as_deref(), Some("*"));
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn playse_non_numeric_pan_defaults_to_zero() {
        let s = parse_script("t.sc", ".playSE ryo_yobidashi.ogg[50] true O *\n");
        assert!(s.errors.is_empty(), "{:?}", s.errors);
        match &s.commands[0] {
            ScriptCommand::PlaySe(p) => {
                assert!(p.looping);
                assert_eq!(p.pan, 0.0);
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn parses_compat_include() {
        let s = parse_script("t.sc", "#include H004_ero.sc\n");
        assert!(s.errors.is_empty());
        match &s.commands[0] {
            ScriptCommand::Compat(c) => {
                assert_eq!(c.name, "#include");
                assert!(c.known);
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn unknown_command_is_warning_not_error() {
        let s = parse_script("t.sc", ".frobnicate 1 2 3\n");
        assert!(s.errors.is_empty());
        match &s.commands[0] {
            ScriptCommand::Compat(c) => assert!(!c.known),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn stats_count_blank_and_comments() {
        let s = parse_script("t.sc", "; comment\n\n.wait 1\n");
        assert_eq!(s.stats.physical_lines, 3);
        assert_eq!(s.stats.blank_lines, 1);
        assert_eq!(s.stats.comment_lines, 1);
        assert_eq!(s.stats.code_lines, 1);
    }

    #[test]
    fn duplicate_label_is_warning() {
        let s = parse_script("t.sc", ".label x\n.wait 1\n.label x\n");
        assert!(s.errors.is_empty());
        assert_eq!(s.warnings.len(), 1);
    }
}
