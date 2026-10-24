#!/usr/bin/env python3
"""Test the self-referential RC4 key scheme found in FUN_00522AC0.

keybuf[i] = keystr[i % len(keystr)] ^ pad[i]   (i = 0..255)
RC4-KSA(keybuf, 256) -> keystream, then XOR data at absolute offset & 0xFFFF.
"""
import struct, sys, os, itertools
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import rc4 as rc4mod

def validate(buf, strict=True):
    if len(buf) < 8: return None
    count = struct.unpack_from('<I', buf, 0)[0]
    if not (1 <= count <= 20000): return None
    p = 4; names = []
    for _ in range(count):
        z = buf.find(b'\0', p, min(p + 96, len(buf)))
        if z < 0: return None
        nm = buf[p:z]
        if not nm or any(c < 0x20 or c > 0x7e for c in nm): return None
        names.append(nm); p = z + 1 + 24
        if p > len(buf): return None
    if strict and len(buf) - p > 16: return None
    return names

def ksa(key):
    S = list(range(256)); j = 0
    for i in range(256):
        j = (j + S[i] + key[i % len(key)]) & 0xff
        S[i], S[j] = S[j], S[i]
    return S

def prga(S, n):
    out = bytearray(); i = j = 0
    for _ in range(n):
        i = (i + 1) & 0xff; j = (j + S[i]) & 0xff
        S[i], S[j] = S[j], S[i]
        out.append(S[(S[i] + S[j]) & 0xff])
    return bytes(out)

def keystr_candidates(name, ident, idxlen, fsize):
    out = set()
    for base in (name, name.upper(), name.lower(), ident, ident.upper(), ident.lower()):
        for n in (idxlen, fsize, fsize - 0x24 - idxlen, 0x24 + idxlen, 0, 1):
            for w in (8, 4):
                for fmt in ('%0*x', '%0*X'):
                    h = (fmt % (w, n)).encode()
                    out.add(base + b' ' + h)
                    out.add(base + h)
                    out.add(base + b' ' + h + b'\0')
    return [k for k in out if 0 < len(k) <= 256]

def test(path, ident):
    d = open(path, 'rb').read()
    idxlen = struct.unpack_from('<I', d, 0x20)[0]
    idx = d[0x24:0x24 + idxlen]
    name = os.path.basename(path).encode()[:-4]
    print('== %s idxlen=%d' % (path, idxlen))
    hits = 0
    for ks_str in keystr_candidates(name, ident, idxlen, len(d)):
        pad = idx + bytes(256)  # pad from ciphertext; zero-extend
        pad = pad[:256]
        keybuf = bytes(ks_str[i % len(ks_str)] ^ pad[i] for i in range(256))
        S = ksa(keybuf)
        stream = prga(S, min(0x10000, idxlen + 0x24 + 16))
        for base in (0, 0x24):
            out = bytes(c ^ stream[(base + i) % 0x10000] for i, c in enumerate(idx))
            r = validate(out)
            if r:
                print('  HIT keystr=%r base=%d names=%r' % (ks_str, base, r[:6]))
                hits += 1
    print('  hits', hits)

test('samples/main/scr.paz', b'eden_en')
test('/mnt/f/gal/【PC硬盘】【官中】eden/sys.paz', b'eden_en')
test('samples/pm/scr.paz', b'eden_PlusMosaic_en')
