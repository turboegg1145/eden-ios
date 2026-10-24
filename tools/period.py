import sys
def an(path,start,end):
    d=open(path,'rb').read()[start:end]
    n=len(d)
    print(f"### {path} [{start:#x}:{end:#x}] n={n}")
    rows=[]
    for p in range(1,min(257,n)):
        z=sum(1 for i in range(n-p) if d[i]==d[i+p])
        rows.append((z/(n-p),p))
    rows.sort(reverse=True)
    for f,p in rows[:12]:
        print(f"  shift={p:3d} same={f:.3f}")
    print()
for p,s,e in (("samples/main/scr.paz",0x24,0x30C),("samples/main/st.paz",0x24,0x64),
              ("samples/pm/scr.paz",0x24,0x24+1160),("samples/pm/st.paz",0x24,0x64),
              ("samples/main/scr.paz",0x30C,0x30C+0x400)):
    an(p,s,e)
