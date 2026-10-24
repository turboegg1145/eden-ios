#!/usr/bin/env python3
"""Decrypt a minori Musica .paz index and dump its entries.

Findings (REA + Unicorn verified):
  * index cipher : Blowfish ECB, 32-byte key from exe .data table
  * key transform: key[i] = (-table[slot][i]) & 0xFF   (vtable[3] negates)
  * block words  : little-endian u32 pair
  * record       : 6 u32 after the NUL-terminated name
"""
import struct, sys, os, json

SECS_PM = [(0x401000, 0x400, 0x147a00), (0x549000, 0x147e00, 0x4bc00),
           (0x595000, 0x193a00, 0x5400), (0x5d2000, 0x198e00, 0x90a00)]
SECS_MAIN = [(0x401000, 0x1000, 0x146200), (0x548000, 0x148000, 0x4ba00),
             (0x594000, 0x194000, 0x5200), (0x5d1000, 0x1d1000, 0x90400),
             (0x662000, 0x262000, 0xb000)]


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
        out = bytearray()
        for i in range(0, len(data) - len(data) % 8, 8):
            L, R = struct.unpack_from('<II', data, i)
            L, R = self._dec(L, R)
            out += struct.pack('<II', L, R)
        return bytes(out)


def key_for(exe_path, slot, secs):
    d = open(exe_path, 'rb').read()
    raw = read_va(d, 0x598478 + slot * 0x20, secs)[:0x20]
    return bytes((256 - b) & 0xff for b in raw), raw


def parse_index(buf):
    n = struct.unpack_from('<I', buf, 0)[0]
    p = 4
    out = []
    for _ in range(n):
        z = buf.index(b'\0', p)
        name = buf[p:z].decode('cp932', 'replace')
        p = z + 1
        rec = struct.unpack_from('<6I', buf, p)
        p += 24
        out.append((name, rec))
    return out


def main():
    exe, paz = sys.argv[1], sys.argv[2]
    secs = SECS_MAIN if 'PLUS' not in exe and 'pm' not in exe else SECS_PM
    d = open(exe, 'rb').read()
    # slot table from DAT_005986F8 (same in both builds)
    names = ['scr', 'bg', 'bg', 'st', 'sys', 'bgm', 'voice', 'se', 'se', 'mov']
    base = os.path.basename(paz).lower()
    stem = base.split('.')[0]
    slot = None
    for i, nm in enumerate(names):
        if nm == stem:
            slot = i
    if slot in (2, 8):
        slot -= 1
    if slot is None:
        raise SystemExit('unknown archive %r' % stem)
    raw = open(paz, 'rb').read()
    idxlen = struct.unpack_from('<I', raw, 0x20)[0]
    ident = raw[:0x20].split(b'\0')[0]
    key, kraw = key_for(exe, slot, secs)
    idx = raw[0x24:0x24 + idxlen]
    P, S = load_tables(exe)
    plain = BF(key, P, S).dec(idx)
    entries = parse_index(plain)
    print('%s  id=%r idxlen=%d entries=%d  key=%s' %
          (paz, ident, idxlen, len(entries), kraw.hex()))
    for name, rec in entries[:int(os.environ.get('N', '20'))]:
        print('   %-28s %s' % (name, ' '.join('%d' % x for x in rec)))
    if os.environ.get('JSON'):
        json.dump([[n, list(r)] for n, r in entries], open(os.environ['JSON'], 'w'))


if __name__ == '__main__':
    main()
