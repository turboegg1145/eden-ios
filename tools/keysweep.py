import sys

def score(buf):
    s=0
    for c in buf:
        if c==0: s+=3
        elif 0x20<=c<0x7f: s+=2
    return s/len(buf)

def sweep(seg,label,maxp=64):
    n=len(seg)
    best=[]
    for p in range(1,maxp+1):
        key=bytearray(p)
        for j in range(p):
            col=[seg[i] for i in range(j,n,p)]
            bk,bs=0,-1
            for k in range(256):
                sc=score(bytes(c^k for c in col))
                if sc>bs: bs,bk=sc,k
            key[j]=bk
        dec=bytes(seg[i]^key[i%p] for i in range(n))
        best.append((score(dec),p,bytes(key),dec))
    best.sort(reverse=True)
    print(f"### {label} n={n}")
    for sc,p,k,dec in best[:5]:
        print(f"  XOR p={p} score={sc:.3f} key={k.hex()}")
        print(f"     {dec[:96]!r}")
    # ADD / SUB
    for op in ('add','sub'):
        best2=[]
        for p in range(1,min(maxp,n)+1):
            key=bytearray(p)
            for j in range(p):
                col=[seg[i] for i in range(j,n,p)]
                bk,bs=0,-1
                for k in range(256):
                    if op=='add': cand=bytes((c-k)&0xff for c in col)
                    else: cand=bytes((c+k)&0xff for c in col)
                    sc=score(cand)
                    if sc>bs: bs,bk=sc,k
                key[j]=bk
            if op=='add': dec=bytes((seg[i]-key[i%p])&0xff for i in range(n))
            else: dec=bytes((seg[i]+key[i%p])&0xff for i in range(n))
            best2.append((score(dec),p,bytes(key),dec))
        best2.sort(reverse=True)
        for sc,p,k,dec in best2[:3]:
            print(f"  {op.upper()} p={p} score={sc:.3f} key={k.hex()}")
            print(f"     {dec[:96]!r}")
    print()

for f,start,ln in (("samples/main/scr.paz",0x24,744),("samples/pm/scr.paz",0x24,1160),("samples/main/st.paz",0x24,40)):
    d=open(f,'rb').read()
    sweep(d[start:start+ln], f)
