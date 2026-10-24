#!/usr/bin/env python3
"""Robust MSVC RTTI/vtable locator for PE32 (handles raw!=va and RVA/VA variants)."""
import struct, sys, json

class PE:
    def __init__(self, path):
        self.data = open(path, 'rb').read()
        d = self.data
        e = struct.unpack_from('<I', d, 0x3c)[0]
        coff = e + 4
        self.nsec = struct.unpack_from('<H', d, coff+2)[0]
        optsz = struct.unpack_from('<H', d, coff+16)[0]
        opt = coff + 20
        self.image_base = struct.unpack_from('<I', d, opt+28)[0]
        self.size_of_image = struct.unpack_from('<I', d, opt+56)[0]
        sec = opt + optsz
        self.sections = []
        for i in range(self.nsec):
            name = d[sec+i*40:sec+i*40+8].rstrip(b'\0').decode('latin1')
            vsize, vaddr, rawsize, rawptr = struct.unpack_from('<IIII', d, sec+i*40+8)
            self.sections.append((name, vaddr, vsize, rawptr, rawsize))
        self.sections.sort(key=lambda s: s[1])
    def off2rva(self, off):
        for n, va, vs, rp, rs in self.sections:
            if rp <= off < rp + rs:
                return va + (off - rp)
        return None
    def rva2off(self, rva):
        for n, va, vs, rp, rs in self.sections:
            if va <= rva < va + max(vs, rs):
                return rp + (rva - va)
        return None
    def u32off(self, off):
        return struct.unpack_from('<I', self.data, off)[0]

def find(pe, cls):
    d = pe.data
    needle = ('.?AV%s@@' % cls).encode()
    i = d.find(needle)
    if i < 0: return None
    name_rva = pe.off2rva(i)
    td_rva = name_rva - 8
    td_va = pe.image_base + td_rva
    res = dict(cls=cls, name_rva=name_rva, td_rva=td_rva, cols=[], vtables=[])
    # COL may reference TD by RVA or VA
    for patval, kind in ((td_rva, 'rva'), (td_va, 'va')):
        pat = struct.pack('<I', patval)
        pos = 0
        while True:
            pos = d.find(pat, pos)
            if pos < 0: break
            col_off = pos - 12
            if col_off >= 0:
                sig = pe.u32off(col_off)
                if sig in (0, 1):
                    col_rva = pe.off2rva(col_off)
                    if col_rva is not None:
                        res['cols'].append((col_rva, kind))
            pos += 1
    for col_rva, kind in res['cols']:
        for patval in (col_rva, pe.image_base + col_rva):
            pat = struct.pack('<I', patval)
            pos = 0
            while True:
                pos = d.find(pat, pos)
                if pos < 0: break
                vt_off = pos + 4
                vt_rva = pe.off2rva(vt_off)
                if vt_rva is not None:
                    res['vtables'].append(vt_rva)
                pos += 1
    res['vtables'] = sorted(set(res['vtables']))
    return res

def fns(pe, vt, limit=24):
    out = []
    for i in range(limit):
        off = pe.rva2off(vt + i*4)
        if off is None: break
        v = pe.u32off(off)
        if not (pe.image_base <= v < pe.image_base + pe.size_of_image): break
        out.append(v)
    return out

if __name__ == '__main__':
    pe = PE(sys.argv[1])
    classes = sys.argv[2:] or ['ZArchiveFileReader','ZArchiveFileReader2','ZArchiveFileReader3',
        'BlowfishEncryptionKey','ScriptEngine','CLayerManager','Animater','LipAnimater','EyeAnimater',
        'UniAnimater','CommandBase','CommandAnimation','StageInfo','CMMV','MMVMotion','ZInflator',
        'ArcFourDummyFileReader','CVorbisPlayer','ScreenEffect','SECrossFade','SERain','SceneGame',
        'CCharLayer','CCharLayerManager','MessagePanel','Parameters','CommandStage','CommandTransition',
        'CommandEffect','CommandMovie','CommandCutin','CommandShakeScreen','CommandScroll','CommandPanel',
        'CommandOverlay','CommandChar','CommandMessage','CommandSelect','CommandSet','CommandIf','CommandWait',
        'CommandFont','CommandPanel','CommandRotateBG','CommandHSV','CommandSqz']
    out = {}
    for c in classes:
        r = find(pe, c)
        if not r:
            print('%-26s NOT FOUND' % c); continue
        vt = []
        for v in r['vtables']:
            vt.append(dict(vt=v, fns=fns(pe, v)))
        r['vt'] = vt
        out[c] = r
        print('%-26s td=%06x vt=%s' % (c, r['td_rva'], [(hex(x['vt']), len(x['fns'])) for x in vt]))
    json.dump(out, open('reports/rtti_%s.json' % ('pm' if 'pm' in sys.argv[1] else 'main'), 'w'), indent=1)
