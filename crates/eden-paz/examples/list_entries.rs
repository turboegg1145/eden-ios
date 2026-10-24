use std::env;

use eden_paz::PazArchive;

fn main() {
    let mut args = env::args().skip(1);
    let path = args.next().expect("archive path");
    let needle = args.next().unwrap_or_default().to_ascii_lowercase();
    let archive = PazArchive::open(&path).unwrap();
    println!(
        "{}: id={} edition={:?} slot={} entries={}",
        path,
        archive.ident(),
        archive.edition(),
        archive.slot(),
        archive.len()
    );
    for entry in archive.entries() {
        if needle.is_empty() || entry.name.to_ascii_lowercase().contains(&needle) {
            println!(
                "{}\t{}\t{}\t{}",
                entry.name, entry.offset, entry.unpacked_size, entry.size
            );
        }
    }
}
