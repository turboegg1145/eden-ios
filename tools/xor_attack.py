import sys, collections, os

def load(p):
    return open(p,'rb').read()

def mtp_attack(buf, start, length, maxper=64, label=""):
    """Many-time-pad: for each period p, key byte at pos j = mode of buf[start+j::p]"""
    res=[]
    seg = buf[start:start+length]
    for p in range(1, maxper+1):
        key=bytearray(p)
        for j in range(p):
            col = seg[j::p]
            if not col: continue
            key[j]=collections.Counter(col).most_common(1)[0][0]
        dec = bytes(seg[i]^key[i%p] for i in range(len(seg)))
        printable = sum(1 for c in dec if 32<=c<127)
        score = printable/len(dec)
        res.append((score,p,bytes(key),dec))
    res.sort(reverse=True)
    print(f"--- MTP {label} start=0x{start:X} len={length} ---")
    for score,p,key,dec in res[:6]:
        print(f"  p={p:2d} printable={score:.3f} key={key.hex()}")
        print(f"     {dec[:80]!r}")
    return res

for f,start,length in (("samples/main/st.paz",0x24,40),
                       ("samples/pm/st.paz",0x24,40),
                       ("samples/main/scr.paz",0x24,744),
                       ("samples/pm/scr.paz",0x24,1160)):
    b=load(f)
    mtp_attack(b,start,length,64,os.path.basename(os.path.dirname(f))+"/"+os.path.basename(f))
    print()
