import struct, sys

class PE:
    def __init__(self, path):
        self.path=path
        self.d=open(path,'rb').read()
        d=self.d
        e_lfanew=struct.unpack_from('<I',d,0x3C)[0]
        assert d[e_lfanew:e_lfanew+4]==b'PE\0\0', 'not PE'
        coff=e_lfanew+4
        nsec=struct.unpack_from('<H',d,coff+2)[0]
        optsz=struct.unpack_from('<H',d,coff+16)[0]
        opt=coff+20
        magic=struct.unpack_from('<H',d,opt)[0]
        assert magic==0x10b, hex(magic)
        self.image_base=struct.unpack_from('<I',d,opt+28)[0]
        self.entry=self.image_base+struct.unpack_from('<I',d,opt+16)[0]
        self.secs=[]
        so=opt+optsz
        for i in range(nsec):
            o=so+i*40
            name=d[o:o+8].rstrip(b'\0').decode('latin1')
            vsz,va,rsz,roff=struct.unpack_from('<IIII',d,o+8)
            self.secs.append(dict(name=name,vsz=vsz,va=va,rsz=rsz,roff=roff))
    def off2va(self,off):
        for s in self.secs:
            if s['roff']<=off<s['roff']+max(s['rsz'],s['vsz']):
                return self.image_base+s['va']+(off-s['roff'])
        return None
    def va2off(self,va):
        r=va-self.image_base
        for s in self.secs:
            if s['va']<=r<s['va']+max(s['rsz'],s['vsz']):
                return s['roff']+(r-s['va'])
        return None
    def sec_of(self,va):
        r=va-self.image_base
        for s in self.secs:
            if s['va']<=r<s['va']+s['vsz']: return s
        return None
    def find(self,pat,start=0):
        out=[];i=start
        while True:
            i=self.d.find(pat,i)
            if i<0: break
            out.append(i); i+=1
        return out
    def xrefs_imm(self,va, sec=None):
        """find 0x68 <va> (push imm32) and c7 44 24 xx <va> (mov [esp+x],imm32) etc."""
        res=[]
        pat=struct.pack('<I',va)
        t = sec or next((s for s in self.secs if s['name']=='.text'), None)
        if not t: return res
        seg=self.d[t['roff']:t['roff']+t['rsz']]
        for m in self.find(pat):
            if not (t['roff']<=m<t['roff']+t['rsz']): continue
            ctx=seg[m-t['roff']-3:m-t['roff']]
            kind=None
            if ctx[0]==0x68: kind='push'
            elif len(ctx)>=3 and ctx[0]==0xC7: kind='mov'
            res.append((self.off2va(m), kind, ctx.hex()))
        return res

if __name__=='__main__':
    pe=PE(sys.argv[1])
    print('image_base',hex(pe.image_base),'entry',hex(pe.entry))
    for s in pe.secs: print(' ',s['name'],hex(s['va']),hex(s['vsz']),hex(s['roff']),hex(s['rsz']))
    for arg in sys.argv[2:]:
        if arg.startswith('str:'):
            pat=arg[4:].encode('latin1')
            print(f'--- str {arg[4:]!r} ---')
            for off in pe.find(pat)[:10]:
                print('   off',hex(off),'va',hex(pe.off2va(off) or 0))
        elif arg.startswith('va:'):
            va=int(arg[3:],16)
            print(f'--- xrefs to {hex(va)} ---')
            for r in pe.xrefs_imm(va): print('  ',hex(r[0]),r[1],r[2])
