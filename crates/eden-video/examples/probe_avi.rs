//! Probe a real AVI: `cargo run -p eden-video --example probe_avi -- <file> [frame_time]`

use eden_video::avi::AviFile;
use eden_video::ffmpeg;

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("avi path");
    let time: f64 = args.next().map(|s| s.parse().unwrap()).unwrap_or(2.0);
    let avi = AviFile::open(&path).unwrap();
    println!(
        "{}: {}x{} {:?} fps={:.4} frames={} duration={:.2}s chunks={} audio={:?}",
        path,
        avi.video.width,
        avi.video.height,
        avi.video.codec,
        avi.video.fps,
        avi.video.frame_count,
        avi.duration(),
        avi.video_chunks.len(),
        avi.audio.as_ref().map(|a| (&a.format.codec, a.format.channels, a.format.sample_rate, a.chunks.len()))
    );
    if let Some(audio) = avi.decode_audio().unwrap() {
        println!(
            "audio: {} Hz {} ch {:.2}s peak={:.4}",
            audio.sample_rate,
            audio.channels,
            audio.duration(),
            audio.peak()
        );
    }
    let frame = ffmpeg::decode_frame_at(&path, time, 1024, 640, avi.video.fps).unwrap();
    let lit = frame
        .data
        .chunks_exact(4)
        .filter(|p| p[0] > 8 || p[1] > 8 || p[2] > 8)
        .count();
    println!("frame @{time}s: {}x{} lit={lit}", frame.width, frame.height);
}
