//! `eden-paz` — pure-Rust Musica `.paz` VFS and asset decoders.
//!
//! * Blowfish (little-endian words, negated per-slot keys)
//! * RC4 streaming (cross-chunk state) and `mov.paz` 64 KiB block XOR
//! * 64-bit offset `pread` streaming; no whole-archive mmap
//! * PNG decode to RGBA8 and `.ani` differential-frame compositing
//! * byte-bounded LRU caches (256 MiB default)

pub mod ani;
pub mod cache;
pub mod crypto;
pub mod error;
pub mod exe;
pub mod image;
pub mod paz;
pub mod tables;

pub use ani::{parse_ani, AniFile, AniFrame};
pub use cache::{ByteLru, WeightedLru};
pub use crypto::{Blowfish, Rc4};
pub use error::{PazError, Result};
pub use exe::{Edition, KeySet, PeImage};
pub use image::{decode_png, encode_png, RgbaImage};
pub use paz::{entry_rc4_key, EntryReader, EntryStream, PazArchive, PazEntry, DEFAULT_CACHE_BYTES};
