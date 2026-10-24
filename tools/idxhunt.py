import struct, sys, itertools
sys.path.insert(0,'tools')
from rc4 import rc4

def validate(buf, verbose=False):
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

def hex8(n, upper=True):
    s = '%08X' % (n & 0xFFFFFFFF)
    return s if upper else s.lower()

names_c = ["", "scr", "scr.paz", "eden_en", "eden_en.scr", "sys", "st", "eden", "SCR",
           "eden_en\\scr", "eden_en/scr", "eden_en_scr", "edenenscr", "eden_en.paz",
           "scr.paz\\", ".paz", "eden_en.scr.paz"]
sizes = [0, 744, 40, 1160, 21376, 759596, 758852, 49646452, 100, 759520, 744+32]
seps = ["", ".", "_", "\\", "/", "-"]

def hunt(path, start, ln, label):
    d = open(path,'rb').read()
    seg = d[start:start+ln]
    hdr32 = d[0:32]; key24 = d[8:32]
    found=[]
    # direct / simple transforms
    for tag, buf in (("plain", seg),
                     ("xor24", bytes(c ^ key24[i%24] for i,c in enumerate(seg))),
                     ("xor32", bytes(c ^ hdr32[i%32] for i,c in enumerate(seg)))):
        r = validate(buf)
        if r: found.append((tag, r[0][:6]))
    # RC4 over candidate keys
    for nm in names_c:
        for sz in sizes:
            for up in (True, False):
                for sep in seps:
                    k = (nm + sep + hex8(sz, up)).encode()
                    ks = rc4(k, ln)
                    out = bytes(a ^ b for a,b in zip(seg, ks))
                    r = validate(out)
                    if r: found.append((f"rc4:{k!r}", r[0][:6]))
    # RC4 with header bytes as key
    for k in (hdr32, key24, hdr32.rstrip(b'\0'), hdr32[:8]):
        ks = rc4(k, ln)
        out = bytes(a^b for a,b in zip(seg,ks))
        r = validate(out)
        if r: found.append((f"rc4hdr:{k[:12]!r}", r[0][:6]))
    print(label, "FOUND", len(found))
    for f in found[:8]: print("   ", f)

hunt('samples/main/scr.paz', 0x24, 744, 'main/scr')
hunt('samples/pm/scr.paz', 0x24, 1160, 'pm/scr')
hunt('samples/main/st.paz', 0x24, 40, 'main/st')
