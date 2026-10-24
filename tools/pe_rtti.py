#!/usr/bin/env python3
"""PE static helper: RTTI/vtable recovery + import table dump for eden.exe.
Pure stdlib. Reads PE32 (image base 0x400000) and maps RVA<->file offset.
"""
import struct, sys, json, os

class PE:
    def __init__(self, path):
        self.path = path
        self.data = open(path, 'rb').read()
        d = self.data
        e_lfanew = struct.unpack_from('<I', d, 0x3c)[0]
        assert d[e_lfanew:e_lfanew+4] == b'PE\0\0'
        coff = e_lfanew + 4
        self.machine, self.nsec, self.tstamp, _, _, self.opt_size, self.chars = \
            struct.unpack_from('<HHIIIHH', d, coff)
        opt = coff + 20
        self.magic = struct.unpack_from('<H', d, opt)[0]
        self.image_base = struct.unpack_from('<I', d, opt+28)[0]
        self.entry = self.image_base + struct.unpack_from('<I', d, opt+16)[0]
        self.size_of_image = struct.unpack_from('<I', d, opt+56)[0]
        self.num_dirs = struct.unpack_from('<I', d, opt+92)[0]
        self.dirs = []
        for i in range(self.num_dirs):
            rva, size = struct.unpack_from('<II', d, opt+96+i*8)
            self.dirs.append((rva, size))
        sec = opt + self.opt_size
        self.sections = []
        for i in range(self.nsec):
            name = d[sec+i*40:sec+i*40+8].rstrip(b'\0').decode('latin1')
            vsize, vaddr, rawsize, rawptr = struct.unpack_from('<IIII', d, sec+i*40+8)
            self.sections.append(dict(name=name, vsize=vsize, vaddr=vaddr,
                                      rawsize=rawsize, rawptr=rawptr,
                                      end=vaddr+max(vsize, rawsize)))
        self.sections.sort(key=lambda s: s['vaddr'])

    def rva2off(self, rva):
        for s in self.sections:
            if s['vaddr'] <= rva < s['end']:
                return s['rawptr'] + (rva - s['vaddr'])
        return None

    def off2rva(self, off):
        for s in self.sections:
            if s['rawptr'] <= off < s['rawptr'] + s['rawsize']:
                return s['vaddr'] + (off - s['rawptr'])
        return None

    def read_rva(self, rva, n):
        o = self.rva2off(rva)
        if o is None: return b''
        return self.data[o:o+n]

    def u32(self, rva):
        b = self.read_rva(rva, 4)
        return struct.unpack('<I', b)[0] if len(b) == 4 else None

    def imports(self):
        if len(self.dirs) < 2: return {}
        rva, size = self.dirs[1]
        out = {}
        off = self.rva2off(rva)
        if off is None: return out
        while True:
            ilt, ts, fc, name_rva, iat = struct.unpack_from('<IIIII', self.data, off)
            if name_rva == 0: break
            dll = self.read_rva(name_rva, 256).split(b'\0')[0].decode('latin1')
            thunk_rva = ilt or iat
            names = []
            toff = self.rva2off(thunk_rva)
            if toff is not None:
                while True:
                    v = struct.unpack_from('<I', self.data, toff)[0]
                    if v == 0: break
                    if not (v & 0x80000000):
                        nm = self.read_rva(v+2, 128).split(b'\0')[0].decode('latin1')
                        names.append(nm)
                    else:
                        names.append('#%d' % (v & 0xffff))
                    toff += 4
            out[dll] = names
            off += 20
        return out


def find_rtti(pe, clsname):
    """Return dict with name_va, td_va, col_va, vtable_va, funcs[]."""
    needle = ('.?AV%s@@' % clsname).encode()
    d = pe.data
    res = {'class': clsname}
    idx = d.find(needle)
    if idx < 0:
        return None
    name_rva = pe.off2rva(idx)
    res['name_rva'] = name_rva
    td_rva = name_rva - 8           # TypeDescriptor: vftable, spare, name
    res['td_rva'] = td_rva
    # find COL referencing td (pTypeDescriptor is RVA at COL+12)
    pat = struct.pack('<I', td_rva)
    pos = 0
    cols = []
    while True:
        pos = d.find(pat, pos)
        if pos < 0: break
        col_rva = pe.off2rva(pos - 12) if pos >= 12 else None
        if col_rva:
            sig = pe.u32(col_rva)
            if sig in (0, 1):
                cols.append(col_rva)
        pos += 1
    res['cols'] = cols
    # find vtables: dword == col_va located right before vtable
    vt = []
    for col in cols:
        col_va = pe.image_base + col
        pat2 = struct.pack('<I', col_va)
        pos = 0
        while True:
            pos = d.find(pat2, pos)
            if pos < 0: break
            rva = pe.off2rva(pos)
            if rva is not None and 0 < rva < pe.size_of_image:
                vt.append(rva + 4)
            pos += 1
    res['vtables'] = sorted(set(vt))
    return res


def vtable_funcs(pe, vt_rva, maxn=120):
    out = []
    for i in range(maxn):
        v = pe.u32(vt_rva + i*4)
        if v is None or v == 0: break
        if v < pe.image_base or v > pe.image_base + pe.size_of_image:
            break
        out.append(v)
    return out


if __name__ == '__main__':
    pe = PE(sys.argv[1] if len(sys.argv) > 1 else 'samples/main/eden.exe')
    print('sections:')
    for s in pe.sections:
        print('  %-10s va=%08x size=%08x raw=%08x' % (s['name'], s['vaddr'], s['vsize'], s['rawptr']))
    print('entry=%08x image_base=%08x dirs=%d' % (pe.entry, pe.image_base, len(pe.dirs)))
    imp = pe.imports()
    print('imports:')
    for dll, names in imp.items():
        print('  %s (%d): %s' % (dll, len(names), ', '.join(names[:40])))
    classes = sys.argv[2:] or ['ZArchiveFileReader','ZArchiveFileReader2','ZArchiveFileReader3',
                               'BlowfishEncryptionKey','ScriptEngine','CLayerManager','Animater',
                               'LipAnimater','EyeAnimater','CommandAnimation','StageInfo',
                               'CMMV','MMVMotion','ZInflator','ArcFourDummyFileReader',
                               'CVorbisPlayer','ScreenEffect','SECrossFade','SERain','SceneGame']
    rttis = {}
    for c in classes:
        r = find_rtti(pe, c)
        if r:
            r['funcs'] = [hex(x) for vt in r['vtables'] for x in vtable_funcs(pe, vt)]
            rttis[c] = r
            print('%s: td=%08x col=%s vt=%s' % (c, r['td_rva'],
                  [hex(x) for x in r['cols']], [hex(x) for x in r['vtables']]))
            print('   funcs:', r['funcs'][:40])
    json.dump(rttis, open('reports/rtti.json','w'), indent=1)
