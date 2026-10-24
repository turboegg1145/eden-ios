//! Pure-Rust Blowfish and RC4 primitives used by the Musica `.paz` container.
//!
//! Two details are specific to minori's engine and are *not* the textbook
//! defaults:
//!
//! * The 32-bit Blowfish words are packed **little-endian** from the byte
//!   stream (`struct.unpack_from('<II', ...)` in the reference unpacker).
//! * The 32-byte key table is negated before use:
//!   `key[i] = (256 - table[i]) & 0xFF`, mirroring `vtable[3] = neg dl`.
//!
//! Everything else is standard Bruce Schneier Blowfish (16 rounds, P/S boxes
//! generated from the digits of pi).

use crate::error::{PazError, Result};
use crate::tables::{P_INIT, S_INIT};

/// Blowfish cipher with the standard P/S boxes.
#[derive(Clone)]
pub struct Blowfish {
    p: [u32; 18],
    s: [[u32; 256]; 4],
}

impl Blowfish {
    /// Build a cipher from a key of 1..=56 bytes.
    pub fn new(key: &[u8]) -> Self {
        assert!(!key.is_empty(), "Blowfish key must not be empty");
        let mut p = P_INIT;
        let mut s = S_INIT;

        let mut j = 0usize;
        for pi in p.iter_mut() {
            let mut v = 0u32;
            for _ in 0..4 {
                v = (v << 8) | u32::from(key[j % key.len()]);
                j += 1;
            }
            *pi ^= v;
        }

        let mut l = 0u32;
        let mut r = 0u32;
        for i in (0..18).step_by(2) {
            let (nl, nr) = Self::encrypt_words_raw(&p, &s, l, r);
            l = nl;
            r = nr;
            p[i] = l;
            p[i + 1] = r;
        }
        for si in 0..4 {
            for i in (0..256).step_by(2) {
                let (nl, nr) = Self::encrypt_words_raw(&p, &s, l, r);
                l = nl;
                r = nr;
                s[si][i] = l;
                s[si][i + 1] = r;
            }
        }

        Self { p, s }
    }

    /// Build a cipher from a raw `.paz` table entry, applying the engine's
    /// `(256 - b) & 0xFF` negation.
    pub fn from_paz_table(raw: &[u8; 32]) -> Self {
        Self::new(&negate_key(raw))
    }

    #[inline]
    fn f(&self, x: u32) -> u32 {
        let a = (x >> 24) as usize;
        let b = ((x >> 16) & 0xff) as usize;
        let c = ((x >> 8) & 0xff) as usize;
        let d = (x & 0xff) as usize;
        (self.s[0][a].wrapping_add(self.s[1][b]) ^ self.s[2][c]).wrapping_add(self.s[3][d])
    }

    #[inline]
    fn encrypt_words_raw(p: &[u32; 18], s: &[[u32; 256]; 4], mut l: u32, mut r: u32) -> (u32, u32) {
        #[inline]
        fn f_raw(s: &[[u32; 256]; 4], x: u32) -> u32 {
            let a = (x >> 24) as usize;
            let b = ((x >> 16) & 0xff) as usize;
            let c = ((x >> 8) & 0xff) as usize;
            let d = (x & 0xff) as usize;
            (s[0][a].wrapping_add(s[1][b]) ^ s[2][c]).wrapping_add(s[3][d])
        }
        for i in 0..16 {
            l ^= p[i];
            r ^= f_raw(s, l);
            std::mem::swap(&mut l, &mut r);
        }
        std::mem::swap(&mut l, &mut r);
        r ^= p[16];
        l ^= p[17];
        (l, r)
    }

    /// Encrypt one 64-bit block supplied as two host-order words.
    pub fn encrypt_words(&self, l: u32, r: u32) -> (u32, u32) {
        Self::encrypt_words_raw(&self.p, &self.s, l, r)
    }

    /// Decrypt one 64-bit block supplied as two host-order words.
    pub fn decrypt_words(&self, mut l: u32, mut r: u32) -> (u32, u32) {
        for i in (2..=17).rev() {
            l ^= self.p[i];
            r ^= self.f(l);
            std::mem::swap(&mut l, &mut r);
        }
        std::mem::swap(&mut l, &mut r);
        r ^= self.p[1];
        l ^= self.p[0];
        (l, r)
    }

    /// Encrypt an 8-byte block, little-endian word order.
    pub fn encrypt_block_le(&self, block: &mut [u8; 8]) {
        let l = u32::from_le_bytes([block[0], block[1], block[2], block[3]]);
        let r = u32::from_le_bytes([block[4], block[5], block[6], block[7]]);
        let (l, r) = self.encrypt_words(l, r);
        block[0..4].copy_from_slice(&l.to_le_bytes());
        block[4..8].copy_from_slice(&r.to_le_bytes());
    }

    /// Decrypt an 8-byte block, little-endian word order.
    pub fn decrypt_block_le(&self, block: &mut [u8; 8]) {
        let l = u32::from_le_bytes([block[0], block[1], block[2], block[3]]);
        let r = u32::from_le_bytes([block[4], block[5], block[6], block[7]]);
        let (l, r) = self.decrypt_words(l, r);
        block[0..4].copy_from_slice(&l.to_le_bytes());
        block[4..8].copy_from_slice(&r.to_le_bytes());
    }

    /// Decrypt a buffer in place. `buf.len()` must be a multiple of 8.
    pub fn decrypt_ecb_inplace(&self, buf: &mut [u8]) -> Result<()> {
        if buf.len() % 8 != 0 {
            return Err(PazError::Crypto(format!(
                "Blowfish ECB buffer length {} is not a multiple of 8",
                buf.len()
            )));
        }
        for chunk in buf.chunks_exact_mut(8) {
            let mut block = [0u8; 8];
            block.copy_from_slice(chunk);
            self.decrypt_block_le(&mut block);
            chunk.copy_from_slice(&block);
        }
        Ok(())
    }

    /// Encrypt a buffer in place. Used by tests and save-file obfuscation.
    pub fn encrypt_ecb_inplace(&self, buf: &mut [u8]) -> Result<()> {
        if buf.len() % 8 != 0 {
            return Err(PazError::Crypto(format!(
                "Blowfish ECB buffer length {} is not a multiple of 8",
                buf.len()
            )));
        }
        for chunk in buf.chunks_exact_mut(8) {
            let mut block = [0u8; 8];
            block.copy_from_slice(chunk);
            self.encrypt_block_le(&mut block);
            chunk.copy_from_slice(&block);
        }
        Ok(())
    }
}

/// `key[i] = (256 - table[i]) & 0xFF` as performed by the engine's
/// `BlowfishEncryptionKey::getKey` virtual method.
pub fn negate_key(table: &[u8; 32]) -> [u8; 32] {
    let mut key = [0u8; 32];
    for (dst, src) in key.iter_mut().zip(table.iter()) {
        *dst = 256u16.wrapping_sub(u16::from(*src)) as u8;
    }
    key
}

/// Stateful RC4 stream cipher.
///
/// The `.paz` RC4 layer is a plain keystream XOR that starts at byte 0 of each
/// entry. Streaming decryption must preserve `i`, `j` and the permutation
/// across chunk boundaries; this type exists for exactly that purpose.
#[derive(Clone)]
pub struct Rc4 {
    s: [u8; 256],
    i: u8,
    j: u8,
}

impl Rc4 {
    pub fn new(key: &[u8]) -> Self {
        assert!(!key.is_empty(), "RC4 key must not be empty");
        let mut s = [0u8; 256];
        for (i, v) in s.iter_mut().enumerate() {
            *v = i as u8;
        }
        let mut j = 0u8;
        for i in 0..256 {
            j = j.wrapping_add(s[i]).wrapping_add(key[i % key.len()]);
            s.swap(i, usize::from(j));
        }
        Self { s, i: 0, j: 0 }
    }

    /// XOR the next `data.len()` keystream bytes into `data`.
    pub fn apply_keystream(&mut self, data: &mut [u8]) {
        for byte in data.iter_mut() {
            self.i = self.i.wrapping_add(1);
            self.j = self.j.wrapping_add(self.s[usize::from(self.i)]);
            self.s.swap(usize::from(self.i), usize::from(self.j));
            let k = self.s[usize::from(self.i)].wrapping_add(self.s[usize::from(self.j)]);
            *byte ^= self.s[usize::from(k)];
        }
    }

    /// Advance the stream without producing output.
    pub fn skip(&mut self, n: usize) {
        let mut scratch = [0u8; 4096];
        let mut left = n;
        while left > 0 {
            let take = left.min(scratch.len());
            self.apply_keystream(&mut scratch[..take]);
            left -= take;
        }
    }
}

/// Generate `n` bytes of RC4 keystream.
pub fn rc4_keystream(key: &[u8], n: usize) -> Vec<u8> {
    let mut out = vec![0u8; n];
    Rc4::new(key).apply_keystream(&mut out);
    out
}

/// XOR `data` with a repeating block (the `mov.paz` 64 KiB scheme).
pub fn xor_repeating(data: &mut [u8], block: &[u8]) {
    assert!(!block.is_empty());
    for (i, b) in data.iter_mut().enumerate() {
        *b ^= block[i % block.len()];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blowfish_standard_vector_zero_key() {
        // Bruce Schneier's test vector: key 0000000000000000,
        // plaintext 0000000000000000 -> ciphertext 4EF997456198DD78.
        let bf = Blowfish::new(&[0u8; 8]);
        let (l, r) = bf.encrypt_words(0, 0);
        assert_eq!(l, 0x4EF9_9745);
        assert_eq!(r, 0x6198_DD78);
        let (dl, dr) = bf.decrypt_words(l, r);
        assert_eq!((dl, dr), (0, 0));
    }

    #[test]
    fn blowfish_roundtrip_le() {
        let bf = Blowfish::from_paz_table(&crate::tables::MAIN_TBL1[0]);
        let original = *b"eden-rs!";
        let mut block = original;
        bf.encrypt_block_le(&mut block);
        assert_ne!(block, original);
        bf.decrypt_block_le(&mut block);
        assert_eq!(block, original);
    }

    #[test]
    fn rc4_standard_vector() {
        // RFC 6229 / classic test vector.
        let mut data = *b"Plaintext";
        Rc4::new(b"Key").apply_keystream(&mut data);
        assert_eq!(
            &data,
            &[0xBB, 0xF3, 0x16, 0xE8, 0xD9, 0x40, 0xAF, 0x0A, 0xD3]
        );
    }

    #[test]
    fn rc4_streaming_matches_oneshot() {
        let key = b"eden-test-key";
        let mut one = vec![0u8; 10_000];
        Rc4::new(key).apply_keystream(&mut one);
        let mut stream = Rc4::new(key);
        let mut chunked = vec![0u8; 10_000];
        for chunk in chunked.chunks_mut(37) {
            stream.apply_keystream(chunk);
        }
        assert_eq!(one, chunked);
    }

    #[test]
    fn rc4_skip_matches_prefix() {
        let key = b"skip-key";
        let mut a = vec![0u8; 500];
        let mut r = Rc4::new(key);
        r.skip(123);
        r.apply_keystream(&mut a);
        let mut full = vec![0u8; 500 + 123];
        Rc4::new(key).apply_keystream(&mut full);
        assert_eq!(a, full[123..]);
    }

    #[test]
    fn negate_key_matches_engine() {
        let mut table = [0u8; 32];
        table[0] = 0x58;
        table[1] = 0xFF;
        let key = negate_key(&table);
        assert_eq!(key[0], 0xA8);
        assert_eq!(key[1], 0x01);
    }
}
