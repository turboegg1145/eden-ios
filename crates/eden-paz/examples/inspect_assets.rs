use eden_paz::PazArchive;

fn main() {
    let root = "/mnt/f/gal/【PC硬盘】【官中】eden";
    let bg = PazArchive::open(format!("{root}/bg.paz")).unwrap();
    let sys = PazArchive::open(format!("{root}/sys.paz")).unwrap();
    for name in [
        "ev006.png",
        "ev007a01.png",
        "ev007a01.ani",
        "ev015a01.png",
        "ev214a01.png",
        "ev229a00_mugai_a.png",
        "ev_add02_up06a.png",
        "ev_add02_up06a.ani",
    ] {
        if let Some(e) = bg.entry(name) {
            if name.ends_with(".png") {
                let img = bg.decode_png(name).unwrap();
                println!(
                    "bg {name}: {}x{} ({} bytes)",
                    img.width, img.height, e.unpacked_size
                );
            } else {
                let ani = bg.parse_ani(name).unwrap();
                println!(
                    "bg {name}: {} frames, labels {:?}",
                    ani.frame_count(),
                    ani.frames
                        .iter()
                        .map(|f| f.label.clone())
                        .collect::<Vec<_>>()
                );
            }
        }
    }
    for name in [
        "msgPanel.png",
        "msgPanel_Omni.png",
        "namePanel.png",
        "fadeSide.png",
        "fadeSwirl.png",
        "fadeGray.png",
    ] {
        if let Some(e) = sys.entry(name) {
            let img = sys.decode_png(name).unwrap();
            println!(
                "sys {name}: {}x{} ({} bytes)",
                img.width, img.height, e.unpacked_size
            );
        }
    }
}
