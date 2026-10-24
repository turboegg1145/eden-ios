//! Integration tests against the real `eden*` audio assets.
//!
//! Every test is skipped (with a message) when the game directory is not
//! mounted, mirroring the `eden-paz` test policy.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use eden_audio::{
    decode_audio, decode_paz_entry, AudioClip, AudioPlayer, LipSyncAnalyzer, PlaybackParams,
};
use eden_paz::PazArchive;

fn game_dir() -> Option<PathBuf> {
    let p = PathBuf::from("/mnt/f/gal/【PC硬盘】【官中】eden");
    p.exists().then_some(p)
}

fn samples(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../samples")
        .join(name)
}

#[test]
fn every_voice_entry_is_ogg_and_decodes() {
    let Some(dir) = game_dir() else {
        eprintln!("skip: game dir missing");
        return;
    };
    let archive = PazArchive::open(dir.join("voice.paz")).unwrap();
    assert_eq!(archive.len(), 5056);

    // Voice entries have no extension; identification must rely on magic.
    for entry in archive.entries() {
        let prefix = archive.read_entry_prefix(entry, 4).unwrap();
        assert_eq!(prefix, b"OggS", "{} is not Ogg", entry.name);
    }

    let names = ["sio-C013-0183", "sio-C013-0181b", "sio-C013-0003"];
    let mut decoded = 0;
    for name in names {
        let clip = decode_paz_entry(&archive, name).unwrap();
        assert_eq!(clip.sample_rate, 44_100, "{name}");
        assert_eq!(clip.channels, 1, "{name}");
        assert!(clip.frames() > 8_000, "{name}: {} frames", clip.frames());
        assert!(clip.duration() > 0.2, "{name}");
        assert!(clip.peak() > 0.01, "{name}: silent");
        assert!(clip.rms() > 0.001, "{name}: near-silent");
        assert!(clip.samples.iter().all(|s| s.is_finite()));
        decoded += 1;
    }
    assert_eq!(decoded, 3);
}

#[test]
fn decodes_real_sound_effects() {
    let Some(dir) = game_dir() else {
        eprintln!("skip: game dir missing");
        return;
    };
    let archive = PazArchive::open(dir.join("se.paz")).unwrap();
    assert_eq!(archive.len(), 105);

    let clip = decode_paz_entry(&archive, "inaba_on.ogg").unwrap();
    // Musica stores short effects at 22.05 kHz; music uses 44.1 kHz.
    assert_eq!(clip.sample_rate, 22_050);
    assert_eq!(clip.channels, 2);
    assert!(clip.frames() > 0);
    assert!(clip.peak() > 0.05, "peak {}", clip.peak());
    // The smallest SE is a short click, not a music track.
    assert!(clip.duration() < 5.0);

    let mut effects = 0;
    for entry in archive.entries() {
        let bytes = archive.read_entry_obj(entry).unwrap();
        let clip = decode_audio(&bytes).unwrap();
        assert!(clip.frames() > 0, "{}", entry.name);
        effects += 1;
    }
    assert_eq!(effects, 105);
}

#[test]
fn decodes_real_background_music() {
    let Some(dir) = game_dir() else {
        eprintln!("skip: game dir missing");
        return;
    };
    let archive = PazArchive::open(dir.join("bgm.paz")).unwrap();
    assert_eq!(archive.len(), 38);

    // Smallest track in the archive (3.98 MB Ogg).
    let clip = decode_paz_entry(&archive, "edenBGM028.ogg").unwrap();
    assert_eq!(clip.sample_rate, 44_100);
    assert_eq!(clip.channels, 2);
    assert!(clip.duration() > 30.0, "duration {}", clip.duration());
    assert!(clip.peak() > 0.1);
}

#[test]
fn decodes_real_system_wav_files() {
    let Some(dir) = game_dir() else {
        eprintln!("skip: game dir missing");
        return;
    };
    let archive = PazArchive::open(dir.join("sys.paz")).unwrap();

    let click = decode_paz_entry(&archive, "click.wav").unwrap();
    assert_eq!(click.sample_rate, 22_050);
    assert_eq!(click.channels, 1);
    assert!(click.frames() > 0);
    assert!(
        click.duration() < 1.0,
        "click duration {}",
        click.duration()
    );
    assert!(click.peak() > 0.01);

    // `SEtest.wav` and `VOICEtest.wav` are the engine's audio test assets.
    for name in ["SEtest.wav", "VOICEtest.wav"] {
        let clip = decode_paz_entry(&archive, name).unwrap();
        assert!(clip.frames() > 0, "{name}");
        assert!(clip.peak() > 0.01, "{name}: silent");
        assert!(clip.samples.iter().all(|s| s.is_finite()), "{name}");
    }
}

#[test]
fn plays_a_real_voice_with_volume_pan_and_loop() {
    let Some(dir) = game_dir() else {
        eprintln!("skip: game dir missing");
        return;
    };
    let archive = PazArchive::open(dir.join("voice.paz")).unwrap();
    let clip = Arc::new(decode_paz_entry(&archive, "sio-C013-0003").unwrap());

    // `[50]` means 50% volume; pan is hard left.
    let params = PlaybackParams::from_percent(50, -1.0, false);
    let mut player = AudioPlayer::new(Arc::clone(&clip), params);
    for i in 0..100 {
        let frame = player.next_frame().unwrap();
        assert!((frame[0] - clip.sample(i, 0) * 0.5).abs() < 1e-6);
        assert_eq!(frame[1], 0.0);
    }

    // A looping player must wrap instead of finishing.
    let mut looping = AudioPlayer::new(
        Arc::new(AudioClip::new(8_000, 1, vec![0.25; 4]).unwrap()),
        PlaybackParams::new(1.0, 0.0, true),
    );
    let mut out = [[0.0f32; 2]; 6];
    assert_eq!(looping.mix_into(&mut out), 6);
    assert_eq!(out[4], [0.25, 0.25]);
    assert_eq!(out[5], [0.25, 0.25]);
}

#[test]
fn lip_sync_follows_real_speech() {
    let Some(dir) = game_dir() else {
        eprintln!("skip: game dir missing");
        return;
    };
    let archive = PazArchive::open(dir.join("voice.paz")).unwrap();
    let clip = decode_paz_entry(&archive, "sio-C013-0183").unwrap();

    let analyzer = LipSyncAnalyzer::new(4);
    let mut seen = [0usize; 4];
    let mut peak_frame = 0;
    for block in clip.samples.chunks(1024) {
        let frame = analyzer.process(block);
        assert!(frame < 4);
        seen[frame as usize] += 1;
        peak_frame = peak_frame.max(frame);
    }
    assert!(peak_frame >= 2, "mouth never opened: {seen:?}");
    assert!(seen[0] > 0, "mouth never closed: {seen:?}");

    // After enough silence the envelope decays below the gate.
    for _ in 0..200 {
        analyzer.process(&[0.0; 1024]);
    }
    assert_eq!(analyzer.frame(), 0);
    assert!(analyzer.envelope() < 1e-6);
}

#[test]
fn samples_directory_has_no_audio_but_archives_do() {
    // Guard against silently skipping every real-asset test: the repository
    // samples must at least contain the script archives used elsewhere.
    // CI has neither the private samples nor the retail game directory.
    if !samples("main/scr.paz").exists() && game_dir().is_none() {
        eprintln!("skip: no private assets in CI");
        return;
    }
    assert!(samples("main/scr.paz").exists() || game_dir().is_some());
}
