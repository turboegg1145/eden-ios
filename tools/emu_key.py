#!/usr/bin/env python3
"""Emulate ZArchiveFileReader3::OpenVolume WITH a real BlowfishEncryptionKey
object wired into reader+0x1EC, so the engine performs its own index
decryption. Dump the resulting plaintext index.

Usage: python3 tools/emu_key.py <exe> <paz> <slot>
   slot = key-table slot (0=scr,1=bg,3=st,4=sys,5=bgm,6=voice,7=se,9=mov)
"""
import struct, sys, os
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from emu import Emu, STACK, STACK_SZ

VT_BLOWFISHKEY = 0x56bb18
TBL1 = 0x598478
TBL2 = 0x5985B8


def read_va(d, va):
    secs = [(0x401000, 0x400, 0x147a00), (0x549000, 0x147e00, 0x4bc00),
            (0x595000, 0x193a00, 0x5400), (0x5d2000, 0x198e00, 0x90a00)]
    for v, o, s in secs:
        if v <= va < v + s:
            return d[o + (va - v):]
    raise ValueError(hex(va))


def make_key_obj(emu, keybytes):
    """Build a BlowfishEncryptionKey: [0]=vt [4]=len [8]=bufferDesc"""
    u = emu.uc
    buf = emu.alloc(len(keybytes))
    u.mem_write(buf, keybytes)
    desc = emu.alloc(0x10)
    u.mem_write(desc, struct.pack('<III', 0, len(keybytes), buf))
    obj = emu.alloc(0xC)
    u.mem_write(obj, struct.pack('<III', VT_BLOWFISHKEY, len(keybytes), desc))
    return obj


def main():
    exe, paz, slot = sys.argv[1], sys.argv[2], int(sys.argv[3], 0)
    emu = Emu(exe)
    emu.basedir = os.path.dirname(paz) or '.'
    name = os.path.splitext(os.path.basename(paz))[0]
    # map paz basename -> volume name the engine will request
    emu.setup_object()
    key = read_va(emu.d, TBL1 + slot * 0x20)[:0x20]
    print('key TBL1[%d] = %s' % (slot, key.hex()))
    kob = make_key_obj(emu, key)
    emu.uc.mem_write(emu.obj + 0x1EC, struct.pack('<I', kob))
    print('reader=%08x keyobj=%08x' % (emu.obj, kob))
    ok = emu.open_volume(name.encode(), 0)
    print('watch hit:', ok)
    for l in emu.log[:10]:
        print(' ', l)
    if ok:
        u = emu.uc
        esi = u.reg_read(0x19)  # ESI
        data = emu.dump(esi, 64)
        print('esi=%08x' % esi)
        print('head:', data.hex())
        print('count =', struct.unpack_from('<I', data, 0)[0])
        # dump whole index buffer
        n = struct.unpack_from('<I', emu.dump(emu.obj + 0x0, 4), 0)[0] if False else 0
        out = '/tmp/idx_%s_%s.bin' % (os.path.basename(exe), name)
        open(out, 'wb').write(emu.dump(esi, 0x20000))
        print('dumped ->', out)


main()
