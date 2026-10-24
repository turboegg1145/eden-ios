#!/usr/bin/env python3
"""Broad sweep over candidate index transforms for minori .paz.

Validates a decrypted index body as:
    u32 count
    repeat count:
        name (printable ASCII, NUL terminated, <=64)
        24-byte record
    (allow trailing padding)
"""
import struct, zlib, bz2, lzma, sys, itertools, os
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import rc4 as rc4mod

ARCH = [
    ('samples/main/st.paz', 'eden_en'),
    ('samples/main/scr.paz', 'eden_en'),
    ('samples/pm/st.paz', 'eden_PlusMosaic_en'),
    ('samples/pm/scr.paz', 'eden_PlusMosaic_en'),
]

def load(p):
    d = open(p, 'rb').read()
    return d, struct.unpack_from('<I', d, 0x20)[0]

def validate(buf, strict=True):
    if len(buf) < 8:
        return None
    count = struct.unpack_from('<I', buf, 0)[0]
    if not (1 <= count <= 20000):
        return None
    p = 4
    names = []
    for _ in range(count):
        z = buf.find(b'\0', p, min(p + 96, len(buf)))
        if z < 0:
            return None
        nm = buf[p:z]
        if len(nm) == 0 or any(c < 0x20 or c > 0x7e for c in nm):
            return None
        names.append(nm)
        p = z + 1 + 24
        if p > len(buf):
            return None
    if strict and len(buf) - p > 16:
        return None
    return names

def rc4ks(key, n):
    return rc4mod.rc4(key, n)

def variants(idxlen, fsize, name, garbage):
    """candidate key strings"""
    v = set()
    nums = [idxlen, fsize, fsize - 0x24 - idxlen, 0x24 + idxlen]
    for base in (name, name.upper(), name.lower(),
                 b'eden_en', b'EDEN_EN', b'eden_PlusMosaic_en', b'EDEN_PLUSMOSAIC_EN'):
        v.add(base)
        v.add(base + b'\0')
        for n in nums:
            for w in (8, 4):
                h = ('%0*x' % (w, n)).encode()
                v.add(base + b' ' + h)
                v.add(base + b' ' + h.upper())
                v.add(base + h)
    for n in nums:
        v.add(('%08x' % n).encode())
        v.add(('%08X' % n).encode())
    v.add(garbage)
    v.add(garbage[:8]); v.add(garbage[:16]); v.add(garbage[8:])
    return [k for k in v if 0 < len(k) <= 256]

def main():
    hits = 0
    for path, ident in ARCH:
        d, idxlen = load(path)
        if idxlen <= 0 or 0x24 + idxlen > len(d):
            print('skip', path, idxlen); continue
        idx = d[0x24:0x24 + idxlen]
        body = d[0x24:0x24 + idxlen]
        garbage = d[0x08:0x20]
        fsize = len(d)
        name = os.path.basename(path).encode()[:-4]
        print('== %s idxlen=%d size=%d head=%s' % (path, idxlen, fsize, idx[:16].hex()))

        # 1) compression
        for label, fn in (('zlib', lambda b: zlib.decompress(b)),
                          ('raw', lambda b: zlib.decompress(b, -15)),
                          ('bz2', lambda b: bz2.decompress(b)),
                          ('lzma', lambda b: lzma.decompress(b))):
            try:
                out = fn(body)
                print('   %s -> %d bytes' % (label, len(out)))
            except Exception:
                pass

        keys = variants(idxlen, fsize, name, garbage)
        print('   %d key candidates' % len(keys))
        # 2) RC4 stream XOR, offset base 0 or 0x24
        for k in keys:
            for pad in (0, 1):
                kk = k + b'\xff' * (256 - len(k)) if pad else k
                if len(kk) > 256: continue
                ks = rc4ks(kk, 0x10000)
                for base in (0x24, 0):
                    out = bytes(c ^ ks[(base + i) & 0xffff] for i, c in enumerate(idx))
                    r = validate(out)
                    if r:
                        print('   HIT rc4 key=%r pad=%d base=%d names=%r' % (k, pad, base, r[:5])); hits += 1
        # 3) Blowfish
        try:
            import blowfish as bfmod
            P, S = bfmod.load_tables('samples/pm/eden_pm_en.exe')
            for k in keys:
                for mode in ('ecb', 'cbc'):
                    try:
                        if mode == 'ecb':
                            out = bfmod.ecb_dec(idx, k, P, S)
                        else:
                            out = bfmod.cbc_dec(idx, k, b'\0' * 8, P, S)
                    except Exception:
                        continue
                    r = validate(out)
                    if r:
                        print('   HIT bf-%s key=%r names=%r' % (mode, k, r[:5])); hits += 1
        except Exception as e:
            print('   blowfish unavailable:', e)
    print('total hits', hits)

main()
