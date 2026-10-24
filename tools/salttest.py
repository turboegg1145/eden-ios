#!/usr/bin/env python3
"""Index decryption attempt using the real key formula:
   keystr = NAME + ' ' + hex8(value) + ' ' + SALT
   keybuf[i] = keystr[i % len] ^ pad[i]      (i=0..255)
   RC4-KSA(keybuf,256) -> 64K stream, XOR at (offset & 0xFFFF)
"""
import struct, sys, os
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

SALTS = [b'N426Fd94', b'ol0lOrAf', b'p37j344s', b'Uyiu4Ruy', b'', b'-dd', b'cgv.dat']

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

def candidates(name, ident, idxlen, fsize):
    out = []
    bases = {name, name.upper(), name.lower(), ident, ident.upper(), ident.lower()}
    for base in bases:
        for n in (idxlen, fsize, fsize - 0x24 - idxlen, 0x24 + idxlen, 0, 1):
            for h in (b'%08x' % n, b'%08X' % n, b'%x' % n):
                for salt in SALTS:
                    core = base + b' ' + h
                    out.append(core + (b' ' + salt if salt else b''))
                    out.append(core)
    return out

def run(path, ident):
    d = open(path, 'rb').read()
    idxlen = struct.unpack_from('<I', d, 0x20)[0]
    idx = d[0x24:0x24 + idxlen]
    name = os.path.basename(path).encode()[:-4]
    print('== %s idxlen=%d' % (path, idxlen))
    if idxlen < 256:
        print('  (too short for self-pad)')
        return
    hits = 0
    for keystr in candidates(name, ident, idxlen, len(d)):
        for padmode in ('self', 'zero', 'hdr'):
            if padmode == 'self': pad = idx[:256]
            elif padmode == 'zero': pad = bytes(256)
            else: pad = (d[0x08:0x20] + bytes(256))[:256]
            keybuf = bytes(keystr[i % len(keystr)] ^ pad[i] for i in range(256))
            S = ksa(keybuf)
            stream = prga(S, min(0x10000, idxlen + 0x24))
            for base in (0, 0x24):
                out = bytes(c ^ stream[(base + i) % len(stream)] for i, c in enumerate(idx))
                r = validate(out)
                if r:
                    print('  HIT %r pad=%s base=%d -> %r' % (keystr, padmode, base, r[:6]))
                    hits += 1
    print('  hits', hits)

run('samples/main/scr.paz', b'eden_en')
run('/mnt/f/gal/【PC硬盘】【官中】eden/sys.paz', b'eden_en')
run('samples/pm/scr.paz', b'eden_PlusMosaic_en')
