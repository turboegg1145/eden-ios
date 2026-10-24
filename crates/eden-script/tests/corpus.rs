//! Real-corpus tests: all 58 shipped `.sc` files (23 main + 35 PLUS+MOSAIC).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use eden_script::{
    parse_script_bytes, MemoryScripts, ScriptCommand, ScriptEncoding, ScriptProvider, ScriptVm,
    VmEvent, VmState,
};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn script_dirs() -> Vec<(PathBuf, ScriptEncoding)> {
    vec![
        (root().join("extracted/main_scr"), ScriptEncoding::Gbk),
        (root().join("extracted/pm_scr"), ScriptEncoding::Cp932),
    ]
}

fn load_all() -> (Vec<eden_script::Script>, BTreeMap<String, usize>) {
    let mut scripts = Vec::new();
    let mut counts = BTreeMap::new();
    for (dir, enc) in script_dirs() {
        if !dir.exists() {
            eprintln!("skip: {} missing", dir.display());
            continue;
        }
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().and_then(|e| e.to_str()) != Some("sc") {
                continue;
            }
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            let bytes = std::fs::read(&path).unwrap();
            let script = parse_script_bytes(name.clone(), &bytes, enc)
                .unwrap_or_else(|e| panic!("{name}: decode failed: {e}"));
            assert!(
                script.errors.is_empty(),
                "{name}: {} parse errors: {:?}",
                script.errors.len(),
                script.errors
            );
            for cmd in &script.commands {
                *counts.entry(cmd.name().to_string()).or_insert(0) += 1;
            }
            scripts.push(script);
        }
    }
    (scripts, counts)
}

#[test]
fn all_58_scripts_parse_with_zero_errors() {
    if !script_dirs().iter().all(|(dir, _)| dir.exists()) {
        eprintln!("skip: extracted corpus missing (private game data)");
        return;
    }
    let (scripts, counts) = load_all();
    assert_eq!(scripts.len(), 58, "expected 23 main + 35 PM scripts");

    let code_lines: usize = scripts.iter().map(|s| s.stats.code_lines).sum();
    assert_eq!(
        code_lines, 40_397,
        "the corpus has exactly 40,397 code lines"
    );

    // Exact command census from the shipped data.
    let expected: &[(&str, usize)] = &[
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
    for (name, n) in expected {
        assert_eq!(counts.get(*name).copied().unwrap_or(0), *n, "{name} count");
    }
    // 24 real commands + the `#include` compatibility command.
    assert_eq!(counts.len(), 25);

    // The only compatibility command in the corpus is `#include`.
    let mut compat = BTreeMap::new();
    for s in &scripts {
        for c in &s.commands {
            if let ScriptCommand::Compat(c) = c {
                *compat.entry(c.name.clone()).or_insert(0usize) += 1;
                assert!(c.known, "unknown command {} in {}", c.name, s.name);
            }
        }
    }
    assert_eq!(compat.get("#include").copied().unwrap_or(0), 8);
    assert_eq!(compat.len(), 1);
}

#[test]
fn every_if_and_goto_label_resolves() {
    let (scripts, _) = load_all();
    for s in &scripts {
        for c in &s.commands {
            let label = match c {
                ScriptCommand::If(i) => Some(&i.label),
                ScriptCommand::Goto(g) => Some(&g.label),
                _ => None,
            };
            if let Some(label) = label {
                assert!(
                    s.labels.contains_key(label),
                    "{}: missing label {label:?}",
                    s.name
                );
            }
        }
    }
}

#[test]
fn gbk_trailing_5c_survives_the_parser() {
    // `.message 1 #乗` in GBK: the final byte of 乗 is 0x5C. Decoding first is
    // what prevents the tokenizer from treating it as an escape.
    let mut bytes = b".message 1 #".to_vec();
    bytes.extend_from_slice(&[0x81, 0x5c]);
    bytes.extend_from_slice(b"\r\n.wait 1\r\n");
    let script = parse_script_bytes("gbk5c.sc", &bytes, ScriptEncoding::Gbk).unwrap();
    assert!(script.errors.is_empty(), "{:?}", script.errors);
    match &script.commands[0] {
        ScriptCommand::Message(m) => assert_eq!(m.text.plain_text(), "乗"),
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn real_a001_script_runs_and_save_load_roundtrips() {
    let (scripts, _) = load_all();
    if !scripts.iter().any(|s| s.name == "A001.sc") {
        eprintln!("skip: A001.sc missing (private game data)");
        return;
    }
    let mut set = MemoryScripts::new();
    for s in scripts {
        set.insert(s);
    }
    let provider: Arc<dyn ScriptProvider> = Arc::new(set);

    let mut vm = ScriptVm::new("A001.sc", provider.clone()).unwrap();
    vm.set_auto_advance_clicks(true);
    for _ in 0..300 {
        vm.tick();
    }
    let save = vm.save();
    let json = save.to_json().unwrap();
    let restored = eden_script::SaveState::from_json(&json).unwrap();
    assert_eq!(save, restored);

    let mut a = ScriptVm::restore(save.clone(), provider.clone()).unwrap();
    let mut b = ScriptVm::restore(restored, provider).unwrap();
    a.set_auto_advance_clicks(true);
    b.set_auto_advance_clicks(true);
    for _ in 0..1700 {
        a.tick();
        b.tick();
    }
    assert_eq!(a.save(), b.save());
    assert!(a.last_error().is_none(), "{:?}", a.last_error());
    assert!(a.message().is_some() || matches!(a.state(), VmState::Ended));
}

#[test]
fn every_script_runs_headless_without_vm_errors() {
    for (dir, enc) in script_dirs() {
        if !dir.exists() {
            continue;
        }
        let set = MemoryScripts::from_dir(&dir, enc).unwrap();
        let provider: Arc<dyn ScriptProvider> = Arc::new(set);
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().and_then(|e| e.to_str()) != Some("sc") {
                continue;
            }
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            let mut vm =
                ScriptVm::new(&name, provider.clone()).unwrap_or_else(|e| panic!("{name}: {e}"));
            vm.set_auto_advance_clicks(true);
            for _ in 0..300 {
                vm.tick();
                for e in vm.events() {
                    if let VmEvent::Error { message, line } = e {
                        panic!("{name}:{line}: {message}");
                    }
                }
            }
            assert!(vm.last_error().is_none(), "{name}: {:?}", vm.last_error());
        }
    }
}
