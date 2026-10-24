import zlib, sys, itertools
def try_region(d, start, ln, label):
    seg=d[start:start+ln]
    hits=[]
    for h in (b'\x78\x9c',b'\x78\x01',b'\x78\xda',b'\x78\x5e'):
        k0=seg[0]^h[0]; k1=seg[1]^h[1]
        for k2 in range(256):
            for k3 in range(256):
                key=bytes([k0,k1,k2,k3])
                buf=bytes(c^key[i&3] for i,c in enumerate(seg))
                try:
                    out=zlib.decompress(buf)
                    hits.append((h.hex(),key.hex(),len(out),out[:80]))
                except Exception:
                    pass
    print(label,'hits',len(hits))
    for x in hits[:5]: print('  ',x)
try_region(open('samples/main/scr.paz','rb').read(),0x24,744,'main scr idx')
try_region(open('samples/main/st.paz','rb').read(),0x24,40,'main st idx')
try_region(open('samples/pm/scr.paz','rb').read(),0x24,1160,'pm scr idx')
