//! Debug helper: decrypt a `.paz` index with every edition/slot combination.
//!
//! Usage: `cargo run -p eden-paz --example dump_index -- <archive.paz>`

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

use eden_paz::crypto::Blowfish;
use eden_paz::exe::{Edition, KeySet};

fn main() {
    let path = std::env::args().nth(1).expect("usage: dump_index <paz>");
    let mut file = File::open(&path).unwrap();
    let mut header = [0u8; 0x24];
    file.read_exact(&mut header).unwrap();
    let ident = String::from_utf8_lossy(&header[..0x20])
        .split('\0')
        .next()
        .unwrap()
        .to_string();
    let idx_len = u32::from_le_bytes(header[0x20..0x24].try_into().unwrap()) as usize;
    let mut cipher = vec![0u8; idx_len];
    file.seek(SeekFrom::Start(0x24)).unwrap();
    file.read_exact(&mut cipher).unwrap();
    println!("{path}: id={ident} idx_len={idx_len}");
    for edition in [Edition::Main, Edition::PlusMosaic] {
        let keys = KeySet::embedded(edition);
        for slot in 0..10 {
            let mut buf = cipher.clone();
            let bf = Blowfish::from_paz_table(keys.index_key(slot));
            if bf.decrypt_ecb_inplace(&mut buf).is_err() {
                continue;
            }
            let count = u32::from_le_bytes(buf[0..4].try_into().unwrap());
            // Print only plausible counts.
            if count < 32 {
                let sample = String::from_utf8_lossy(&buf[4..4 + 48.min(buf.len() - 4)]);
                let printable = sample
                    .chars()
                    .filter(|c| c.is_ascii_graphic() || *c == ' ' || *c == '\0')
                    .count();
                if printable > 20 {
                    println!(
                        "  {:?} slot {slot}: count={count} sample={:?}",
                        edition, sample
                    );
                    if slot == 9 {
                        for off in [4usize, 0x104] {
                            let tail = &buf[off..(off + 96).min(buf.len())];
                            println!("    @{off:#x}: {:02x?}", tail);
                            println!(
                                "    ascii: {:?}",
                                String::from_utf8_lossy(tail)
                            );
                        }
                    }
                }
            }
        }
    }
}
