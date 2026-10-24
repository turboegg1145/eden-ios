#!/usr/bin/env python3
"""minori Musica .paz extractor.

Pipeline (verified against eden.exe / eden_pm_en.exe):

    plaintext = zlib_inflate?( rc4_key( name.lower() + " %08X" % usize + " " + type_salt )
                               [ blowfish_ecb_dec( raw, tbl2[slot] ) ] )[:Size]

  * index   : Blowfish ECB, key = (-tbl1[slot]) & 0xFF, little-endian words
  * data    : Blowfish ECB with tbl2[slot], then an RC4 layer whose password
              depends on the file type (see TYPE_SALT)
  * .wav in a non-audio archive gets no RC4 layer at all
  * mov.paz uses a different scheme (see ArcPAZ.cs MovPazArchive)
"""
import os, struct, sys, zlib, json, mmap

SECS_PM = [(0x401000, 0x400, 0x147a00), (0x549000, 0x147e00, 0x4bc00),
           (0x595000, 0x193a00, 0x5400), (0x5d2000, 0x198e00, 0x90a00)]
SECS_MAIN = [(0x401000, 0x1000, 0x146200), (0x548000, 0x148000, 0x4ba00),
             (0x594000, 0x194000, 0x5200), (0x5d1000, 0x1d1000, 0x90400),
             (0x662000, 0x262000, 0xb000)]

TBL1_MAIN, TBL1_PM = 0x597478, 0x598478
TBL2_MAIN, TBL2_PM = 0x5975B8, 0x5985B8

# TypeKeys.  Both binaries keep four consecutive 8-char literals in .rdata and
# copy their addresses into a 4-entry pointer table during startup; the order is
# {sc, ogg, png, avi} (matches GARbro's GetTypePassword dispatch).
#   PM   : 0x56BCDC "N426Fd94" "ol0lOrAf" "p37j344s" "Uyiu4Ruy"
#   main : 0x56AC64 "63JwEajj" "wCCuLx5S" "J1WaoGBi" "F73xvHdo"
SALTS_PM = {'sc': b'N426Fd94', 'ogg': b'ol0lOrAf',
            'png': b'p37j344s', 'avi': b'Uyiu4Ruy'}
SALTS_MAIN = {'sc': b'63JwEajj', 'ogg': b'wCCuLx5S',
              'png': b'J1WaoGBi', 'avi': b'F73xvHdo'}
ALL_SALTS = list(SALTS_PM.values())

ARCHIVE_SLOT = {'scr': 0, 'bg': 1, 'st': 3, 'sys': 4, 'bgm': 5,
                'voice': 6, 'se': 7, 'mov': 9}
AUDIO_ARCHIVES = {'bgm', 'se', 'voice', 'pmbgm', 'pmse', 'pmvoice'}


def read_va(d, va, secs):
    for v, o, s in secs:
        if v <= va < v + s:
            return d[o + (va - v):]
    raise ValueError('va %08x not mapped' % va)


def load_tables(exe_path):
    d = open(exe_path, 'rb').read()
    off = d.find(struct.pack('<I', 0x243F6A88))
    assert off > 0
    vals = struct.unpack_from('<1042I', d, off)
    P = list(vals[:18])
    S = [list(vals[18 + i * 256:18 + (i + 1) * 256]) for i in range(4)]
    return P, S


class BF:
    def __init__(self, key, P0, S0):
        self.P = list(P0)
        self.S = [list(x) for x in S0]
        j = 0
        for i in range(18):
            v = 0
            for _ in range(4):
                v = ((v << 8) | key[j % len(key)]) & 0xffffffff
                j += 1
            self.P[i] ^= v
        L = R = 0
        for i in range(0, 18, 2):
            L, R = self._enc(L, R)
            self.P[i], self.P[i + 1] = L, R
        for s in range(4):
            for i in range(0, 256, 2):
                L, R = self._enc(L, R)
                self.S[s][i], self.S[s][i + 1] = L, R

    def _F(self, x):
        S = self.S
        return ((((S[1][(x >> 16) & 0xff] + S[0][(x >> 24) & 0xff]) & 0xffffffff)
                 ^ S[2][(x >> 8) & 0xff]) + S[3][x & 0xff]) & 0xffffffff

    def _enc(self, L, R):
        P = self.P
        for i in range(16):
            L ^= P[i]
            R ^= self._F(L)
            L, R = R, L
        L, R = R, L
        R ^= P[16]
        L ^= P[17]
        return L & 0xffffffff, R & 0xffffffff

    def _dec(self, L, R):
        P = self.P
        for i in range(17, 1, -1):
            L ^= P[i]
            R ^= self._F(L)
            L, R = R, L
        L, R = R, L
        R ^= P[1]
        L ^= P[0]
        return L & 0xffffffff, R & 0xffffffff

    def dec(self, data):
        n = len(data) - len(data) % 8
        out = bytearray(n)
        for i in range(0, n, 8):
            L, R = struct.unpack_from('<II', data, i)
            L, R = self._dec(L, R)
            struct.pack_into('<II', out, i, L, R)
        return bytes(out)


def rc4_stream(key, n):
    Sx = list(range(256))
    j = 0
    kl = len(key)
    for i in range(256):
        j = (j + Sx[i] + key[i % kl]) & 0xff
        Sx[i], Sx[j] = Sx[j], Sx[i]
    out = bytearray(n)
    i = j = 0
    for k in range(n):
        i = (i + 1) & 0xff
        j = (j + Sx[i]) & 0xff
        Sx[i], Sx[j] = Sx[j], Sx[i]
        out[k] = Sx[(Sx[i] + Sx[j]) & 0xff]
    return bytes(out)


def xor(a, b):
    return bytes(x ^ y for x, y in zip(a, b))


def xor_repeat(data, block):
    """XOR a large buffer with a repeating block (fast big-int path)."""
    n = len(data)
    if not n:
        return b''
    b = (block * (n // len(block) + 1))[:n]
    return (int.from_bytes(data, 'little') ^ int.from_bytes(b, 'little')).to_bytes(n, 'little')


def parse_index(buf, video=False):
    n = struct.unpack_from('<I', buf, 0)[0]
    p = 4
    vkey = None
    if video:
        vkey = bytes(buf[4:4 + 0x100])
        p = 4 + 0x100
    out = []
    for _ in range(n):
        z = buf.index(b'\0', p)
        name = buf[p:z].decode('cp932', 'replace')
        p = z + 1
        off = struct.unpack_from('<Q', buf, p)[0]
        usize, size, aligned, packed = struct.unpack_from('<IIII', buf, p + 8)
        p += 24
        out.append((name, off, usize, size, aligned, packed))
    return vkey, out


class Archive:
    def __init__(self, exe, paz_path, slot=None):
        self.exe = exe
        self.paz_path = paz_path
        self.secs = SECS_MAIN if 'eden.exe' in os.path.basename(exe) else SECS_PM
        self.tbl1 = TBL1_MAIN if self.secs is SECS_MAIN else TBL1_PM
        self.tbl2 = TBL2_MAIN if self.secs is SECS_MAIN else TBL2_PM
        self.salts = SALTS_MAIN if self.secs is SECS_MAIN else SALTS_PM
        stem = os.path.basename(paz_path).split('.')[0].lower()
        self.slot = ARCHIVE_SLOT[stem] if slot is None else slot
        self.is_audio = stem in AUDIO_ARCHIVES
        self.is_video = stem == 'mov'
        self.P, self.S = load_tables(exe)
        self.d = open(exe, 'rb').read()
        if os.path.getsize(paz_path) > 64 << 20:
            self._f = open(paz_path, 'rb')
            self.raw = mmap.mmap(self._f.fileno(), 0, access=mmap.ACCESS_READ)
        else:
            self._f = None
            self.raw = open(paz_path, 'rb').read()
        self.ident = self.raw[:0x20].split(b'\0')[0]
        idxlen = struct.unpack_from('<I', self.raw, 0x20)[0]
        k1 = self._key(self.tbl1)
        idx = BF(k1, self.P, self.S).dec(self.raw[0x24:0x24 + idxlen])
        self.video_key, self.entries = parse_index(idx, self.is_video)
        self.k2 = self._key(self.tbl2)
        self.bf2 = BF(self.k2, self.P, self.S)

    def _key(self, base):
        raw = read_va(self.d, base + self.slot * 0x20, self.secs)[:0x20]
        return bytes((256 - b) & 0xff for b in raw)

    def password(self, name):
        ext = os.path.splitext(name)[1].lower().lstrip('.')
        if self.is_audio:
            return self.salts.get('ogg')
        return self.salts.get(ext)      # unknown ext -> no RC4 layer at all

    def decrypt(self, e, pwd=None, raw_stage=False):
        name, off, usize, size, aligned, packed = e
        blob = self.raw[off:off + aligned]
        if self.is_video:
            # MovPazArchive: no Blowfish.  key[i] = video_key[i] ^ cp932(entry_key)[i%len]
            # then RC4 keystream of 0x10000 bytes XORed cyclically over the stream.
            ek = ('%s %08X ' % (name.lower(), usize)).encode('cp932') + self.salts['avi']
            k = bytes(self.video_key[i] ^ ek[i % len(ek)] for i in range(0x100))
            block = rc4_stream(k, 0x10000)
            return xor_repeat(blob, block)[:size]
        data = self.bf2.dec(blob)
        if pwd is None:
            pwd = self.password(name)
        if pwd:
            key = ('%s %08X ' % (name.lower(), usize)).encode('cp932') + pwd
            data = xor(data, rc4_stream(key, len(data)))
        data = data[:size]
        if packed:
            data = zlib.decompress(data)
        return data


def sniff(b):
    if b[:4] == b'\x89PNG':
        return 'png'
    if b[:4] == b'OggS':
        return 'ogg'
    if b[:4] == b'RIFF':
        return 'wav'
    if b[:2] in (b'\x78\x9c', b'\x78\x01', b'\x78\xda'):
        return 'zlib'
    if b[:4] == b'\x00\x00\x00\x00' or b[:3] == b'RIF':
        return '?'
    printable = sum(1 for c in b[:64] if 32 <= c < 127 or 0xa1 <= c <= 0xdf)
    if printable > 56:
        return 'text'
    return '?'


def main():
    exe, paz_path = sys.argv[1], sys.argv[2]
    outdir = sys.argv[3] if len(sys.argv) > 3 else None
    a = Archive(exe, paz_path)
    print('%s  id=%r  entries=%d  slot=%d' % (paz_path, a.ident, len(a.entries), a.slot))
    stats = {}
    bad = []
    for e in a.entries:
        name, off, usize, size, aligned, packed = e
        data = a.decrypt(e)
        kind = sniff(data)
        stats[kind] = stats.get(kind, 0) + 1
        if kind == '?':
            bad.append((name, usize, data[:16].hex()))
        if outdir:
            p = os.path.join(outdir, name.replace('\\', '/'))
            os.makedirs(os.path.dirname(p) or '.', exist_ok=True)
            open(p, 'wb').write(data)
    print('  ', stats)
    for b in bad[:10]:
        print('   FAIL', b)


if __name__ == '__main__':
    main()
