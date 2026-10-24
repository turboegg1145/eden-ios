import struct, sys, itertools
sys.path.insert(0,'tools')
from blowfish import Blowfish, load_tables
from rc4 import rc4
P,S = load_tables('samples/pm/eden_pm_en.exe')

def validate(buf):
    if len(buf) < 8: return None
    count = struct.unpack_from('<I', buf, 0)[0]
    if not (1 <= count <= 6000): return None
    p = 4; names=[]
    for i in range(count):
        z = buf.find(b'\0', p, p+100)
        if z < 0: return None
        nm = buf[p:z]
        if not (1 <= len(nm) <= 80): return None
        if any(c < 0x20 or c > 0x7e for c in nm): return None
        p = z + 1 + 24
        if p > len(buf): return None
        names.append(nm)
    if len(buf) - p > 8: return None
    return names

def sweep(path, label):
    d = open(path,'rb').read()
    idf = d[0:32]
    hits = []
    keys = set()
    for a in range(0,32):
        for b in range(a+1,33):
            if 1 <= b-a <= 32: keys.add(idf[a:b])
    for base in (b'eden_en', b'scr', b'st', b'sys', b'index', b'list'):
        keys.add(base)
    for k in list(keys):
        keys.add(k.rstrip(b'\0'))
    keys = [k for k in keys if k]
    print(f'{label}: {len(keys)} keys')
    for start,ln,tag in ((0x24, struct.unpack_from('<I',d,0x20)[0], 'idx'),
                         (0x20, struct.unpack_from('<I',d,0x20)[0]+4, 'idx+size')):
        seg = d[start:start+ln]
        if len(seg) < 8: continue
        for k in keys:
            bf = Blowfish(k, P, S)
            for mode in ('dec','enc'):
                fn = bf.decrypt_block if mode=='dec' else bf.encrypt_block
                out = b''.join(fn(seg[i:i+8]) for i in range(0,len(seg)//8*8,8))
                r = validate(out)
                if r: hits.append((tag,mode,'bf',k,r[:4]))
            ks = rc4(k, len(seg))
            out = bytes(a^b for a,b in zip(seg,ks))
            r = validate(out)
            if r: hits.append((tag,'rc4','',k,r[:4]))
    print('  hits', len(hits))
    for h in hits[:10]: print('   ', h)

sweep('samples/main/st.paz','main/st')
sweep('samples/main/scr.paz','main/scr')
