#!/usr/bin/env python3
"""Emulate ZArchiveFileReader3::Open (0x522AC0) RC4 branch and capture the real
256-byte RC4 key + the resulting 64K keystream.

This answers: what exactly seeds the data-area stream cipher?
"""
import struct, sys, os
from unicorn import *
from unicorn.x86_const import *
import emu as E

FAKE_NODE = 0x70001000
END_SENT  = 0x70002000


class EmuR(E.Emu):
    def _hooks(self):
        E.Emu._hooks(self)
        u = self.uc
        u.mem_map(FAKE_NODE & ~0xfff, 0x2000)
        self.rc4_keys = []
        self.rc4_s = []

        def h_find(uc, a, sz, ud):
            self._ret(FAKE_NODE)

        def h_less(uc, a, sz, ud):
            self._ret(0)

        def h_ksa(uc, a, sz, ud):
            # 0x523148: call 0x406c60 ; eax=len, ecx=S, [esp+4]=key
            key = self.arg(0)
            n = uc.reg_read(UC_X86_REG_EAX) & 0xffffffff
            s = uc.reg_read(UC_X86_REG_ECX) & 0xffffffff
            self.rc4_keys.append((key, n, bytes(uc.mem_read(key, n))))
            self.rc4_s.append(s)
            # do NOT stop; let it execute the real KSA

        u.hook_add(UC_HOOK_CODE, h_find, begin=0x407280, end=0x407280)
        u.hook_add(UC_HOOK_CODE, h_less, begin=0x401cd0, end=0x401cd0)
        u.hook_add(UC_HOOK_CODE, h_ksa,  begin=0x406c60, end=0x406c60)

    def setup_reader(self, vol=0):
        """reader3 instance with the two 0x100-byte key buffer descriptors."""
        obj = self.alloc(0x270)
        u = self.uc
        u.mem_write(obj, struct.pack('<I', 0x553054))
        for off in (0x1D0, 0x1D4):
            buf = self.alloc(0x100)          # zeroed key-material buffer
            desc = self.alloc(0x10)
            u.mem_write(desc, struct.pack('<IIII', 0, 0x100, buf, 1))
            u.mem_write(obj + off, struct.pack('<I', desc))
        u.mem_write(obj + 0x1D8, struct.pack('<I', 0))
        u.mem_write(obj + 0x1E0, struct.pack('<I', 0))
        u.mem_write(obj + 0x1F0, struct.pack('<I', 0))
        u.mem_write(obj + 0x1F8, struct.pack('<I', 0))
        u.mem_write(obj + 0x21C, struct.pack('<I', 0))
        u.mem_write(obj + 0x220, struct.pack('<I', 0x20))
        # map object + end sentinel
        u.mem_write(obj + 0xF8, struct.pack('<I', 0x70003000))
        u.mem_write(obj + 0xFC, struct.pack('<I', END_SENT))
        # std::string at +0x1B4 (SSO)
        u.mem_write(obj + 0x1B4, b'\0')
        u.mem_write(obj + 0x1C4, struct.pack('<II', 0, 0xF))
        u.mem_write(END_SENT, struct.pack('<I', END_SENT))
        self.obj = obj
        return obj

    def make_node(self, name, vol, usize, csize, disk_size, flags, keylen=0x20):
        u = self.uc
        nb = name.encode('cp932') if isinstance(name, str) else name
        node = FAKE_NODE
        u.mem_write(node, b'\0' * 0x60)
        # +0x10 : std::string (SSO) holding the name
        u.mem_write(node + 0x10, nb + b'\0')
        u.mem_write(node + 0x20, struct.pack('<II', len(nb), 0xF))
        # +0x30 .. : the 6 index fields + extras
        u.mem_write(node + 0x30, struct.pack('<IIIIIII', vol, usize, csize,
                                            disk_size, flags, 0, keylen))
        return node

    def call_open(self, name, vol=0, stop_at=0x5232A5):
        u = self.uc
        obj = self.obj
        esp = E.STACK + E.STACK_SZ - 0x2000
        nb = name.encode('cp932') if isinstance(name, str) else name
        u.mem_write(esp, struct.pack('<III', 0xdeadbeef, self.strbuf(nb), 0))
        u.reg_write(UC_X86_REG_ESP, esp)
        u.reg_write(UC_X86_REG_ECX, obj)
        u.reg_write(UC_X86_REG_EIP, 0x522AC0)
        hit = {'v': False, 'esp': 0}

        def wcb(uc, a, sz, ud):
            hit['v'] = True
            hit['esp'] = uc.reg_read(UC_X86_REG_ESP)
            uc.emu_stop()
        u.hook_add(UC_HOOK_CODE, wcb, begin=stop_at, end=stop_at)
        try:
            u.emu_start(0x522AC0, 0x5232A5, count=50_000_000)
        except UcError as e:
            print('  emu error:', e, 'eip=%08x' % u.reg_read(UC_X86_REG_EIP))
        return hit


def main():
    exe = sys.argv[1] if len(sys.argv) > 1 else 'samples/pm/eden_pm_en.exe'
    name = sys.argv[2] if len(sys.argv) > 2 else 'uzumaki2.png'
    usize = int(sys.argv[3], 0) if len(sys.argv) > 3 else 464293
    emu = EmuR(exe)
    emu.basedir = 'samples/pm'
    emu.setup_reader()
    emu.make_node(name, 0, usize, usize, usize + 3, 0)
    hit = emu.call_open(name)
    print('reached success:', hit['v'])
    for (p, n, k) in emu.rc4_keys:
        print('RC4 key len=%d @%08x' % (n, p))
        print('  ', k.hex())
        print('  ', k)
    u = emu.uc
    ks = u.reg_read(emu.obj + 0x1D8)
    print('keystream buf @%08x' % ks)
    if ks:
        d = bytes(u.mem_read(ks, 64))
        print('  head:', d.hex())


if __name__ == '__main__':
    main()
