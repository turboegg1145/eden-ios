//! Extract one `.paz` entry verbatim.
//!
//! Usage: `cargo run -p eden-paz --example extract_entry -- <archive.paz> <name> <out>`

use std::io::Write;

use eden_paz::PazArchive;

fn main() {
    let mut args = std::env::args().skip(1);
    let archive_path = args.next().expect("archive path");
    let name = args.next().expect("entry name");
    let out_path = args.next().expect("output path");
    let archive = PazArchive::open(&archive_path).unwrap();
    let bytes = archive.read_entry(&name).unwrap();
    let mut f = std::fs::File::create(&out_path).unwrap();
    f.write_all(&bytes).unwrap();
    eprintln!(
        "{name}: {} bytes -> {out_path}",
        bytes.len()
    );
}
