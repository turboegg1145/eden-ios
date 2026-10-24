import struct, sys
sys.path.insert(0,'tools')
from blowfish import Blowfish, load_tables

def validate(buf):
    if len(buf) < 8: return None
    count = struct.unpack_from('<I', buf, 0)[0]
    if not (1 <= count <= 5000): return None
    p = 4; names=[]; recs=[]
    for i in range(count):
        z = buf.find(b'\0', p, p+100)
        if z < 0: return None
        nm = buf[p:z]
        if not (1 <= len(nm) <= 80): return None
        if any(c < 0x20 or c > 0x7e for c in nm): return None
        p = z + 1
        if p + 24 > len(buf): return None
        recs.append(struct.unpack_from('<6I', buf, p)); p += 24
        names.append(nm.decode('latin1'))
    if len(buf) - p > 8: return None
    return names, recs

P, S = load_tables('samples/pm/eden_pm_en.exe')

def run(path, start, ln, label):
    d = open(path,'rb').read()
    seg = d[start:start+ln]
    idfield = d[0:32]
    ids = idfield.split(b'\0')[0]
    keys = [ids, idfield, d[8:32], ids+b'\0', idfield.rstrip(b'\0'),
            b'eden*', b'EDEN_EN', b'eden_en'*4, idfield*2]
    hits=[]
    for k in keys:
        if not k: continue
        bf = Blowfish(k, P, S)
        out = b''.join(bf.decrypt_block(seg[i:i+8]) for i in range(0, len(seg)//8*8, 8))
        r = validate(out)
        if r: hits.append(('ecb', k[:20], r[0][:6]))
        # cbc iv=0
        prev = b'\0'*8; o2=b''
        for i in range(0, len(seg)//8*8, 8):
            blk = seg[i:i+8]
            o2 += bytes(a^b for a,b in zip(bf.decrypt_block(blk), prev)); prev = blk
        r = validate(o2)
        if r: hits.append(('cbc0', k[:20], r[0][:6]))
    print(label, 'hits', len(hits))
    for h in hits[:6]: print('   ', h)
    # show what the id-keyed ECB gives
    bf = Blowfish(ids, P, S)
    out = b''.join(bf.decrypt_block(seg[i:i+8]) for i in range(0, min(len(seg),64)//8*8, 8))
    print('   sample(id-key ecb):', out[:48].hex())

for a in (('samples/main/st.paz',0x24,40,'main/st'),
          ('samples/main/scr.paz',0x24,744,'main/scr'),
          ('samples/pm/st.paz',0x24,40,'pm/st')):
    run(*a)
