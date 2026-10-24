#!/usr/bin/env python3
"""Emulate minori engine OpenVolume() with the real PE code + hooked Win32 I/O.

Goal: observe exactly what the engine does with the .paz index.
"""
import struct, sys, os
from unicorn import *
from unicorn.x86_const import *

IMG = 0x400000
HEAP = 0x10000000
HEAP_END = 0x14000000
STACK = 0x20000000
STACK_SZ = 0x200000

CREATE = 0x70000001
READ = 0x70000002
SEEK = 0x70000003
NEW = 0x70000010
FREE = 0x70000011
ALLOC = 0x70000012
NOOP = 0x70000013

class Emu:
    def __init__(self, exe):
        self.d = open(exe, 'rb').read()
        self.uc = Uc(UC_ARCH_X86, UC_MODE_32)
        self.files = {}   # handle -> [path, data, pos]
        self.next_h = 0x100
        self.heap = HEAP + 0x100000
        self.log = []
        self._map()
        self._hooks()

    def _map(self):
        u = self.uc
        u.mem_map(0, 0x10000)
        u.mem_map(IMG, 0x300000)
        u.mem_map(HEAP, HEAP_END - HEAP)
        u.mem_map(STACK, STACK_SZ)
        u.mem_map(0x70000000, 0x1000)
        # load PE
        d = self.d
        e = struct.unpack_from('<I', d, 0x3c)[0]
        nsec = struct.unpack_from('<H', d, e + 6)[0]
        optsz = struct.unpack_from('<H', d, e + 20)[0]
        opt = e + 24
        so = opt + optsz
        for i in range(nsec):
            o = so + i * 40
            vsz, va, rsz, roff = struct.unpack_from('<IIII', d, o + 8)
            if rsz:
                u.mem_write(IMG + va, d[roff:roff + rsz])

    def alloc(self, n):
        n = (n + 15) & ~15
        p = self.heap
        self.heap += n
        assert self.heap < HEAP_END, 'heap exhausted'
        self.uc.mem_write(p, b'\0' * n)
        return p

    def _ret(self, val=0, pop=0):
        u = self.uc
        esp = u.reg_read(UC_X86_REG_ESP)
        ra = struct.unpack('<I', u.mem_read(esp, 4))[0]
        u.reg_write(UC_X86_REG_ESP, esp + 4 + pop)
        u.reg_write(UC_X86_REG_EIP, ra)
        u.reg_write(UC_X86_REG_EAX, val & 0xffffffff)

    def arg(self, i):
        u = self.uc
        esp = u.reg_read(UC_X86_REG_ESP)
        return struct.unpack('<I', u.mem_read(esp + 4 * (i + 1), 4))[0]

    def cstr(self, p, n=260):
        try:
            b = bytes(self.uc.mem_read(p, n))
        except Exception:
            return b''
        z = b.find(b'\0')
        return b[:z] if z >= 0 else b

    def _hooks(self):
        u = self.uc
        def hc(addr, size, ud, name):
            def cb(uc, a, sz, ud):
                if a == CREATE:
                    p = self.cstr(self.arg(0))
                    full = p.decode('latin1')
                    local = os.path.join(self.basedir, full)
                    if not os.path.exists(local):
                        self.log.append(('create-fail', full))
                        self._ret(0xffffffff, 28); return
                    data = open(local, 'rb').read()
                    h = self.next_h; self.next_h += 4
                    self.files[h] = [full, data, 0]
                    self.log.append(('create', full, len(data)))
                    self._ret(h, 28); return
                if a == READ:
                    h, buf, n, pread, ov = self.arg(0), self.arg(1), self.arg(2), self.arg(3), self.arg(4)
                    f = self.files.get(h)
                    if not f:
                        self._ret(0, 20); return
                    chunk = f[1][f[2]:f[2] + n]
                    uc.mem_write(buf, chunk)
                    uc.mem_write(pread, struct.pack('<I', len(chunk)))
                    f[2] += len(chunk)
                    self.log.append(('read', h, n, len(chunk), f[2] - len(chunk)))
                    self._ret(1, 20); return
                if a == SEEK:
                    h, lo, phi, method = self.arg(0), self.arg(1), self.arg(2), self.arg(3)
                    f = self.files.get(h)
                    if not f:
                        self._ret(0xffffffff, 16); return
                    if method == 0: f[2] = lo
                    elif method == 1: f[2] += lo
                    elif method == 2: f[2] = len(f[1]) + lo
                    self._ret(f[2], 16); return
                if a == NEW:
                    self._ret(self.alloc(self.arg(0))); return
                if a == FREE:
                    self._ret(0); return
                if a == ALLOC:
                    # 0x4398b0(desc, mode) with size in ECX
                    desc = self.arg(0); size = uc.reg_read(UC_X86_REG_ECX)
                    p = self.alloc(size)
                    uc.mem_write(desc + 4, struct.pack('<I', size))
                    uc.mem_write(desc + 8, struct.pack('<I', p))
                    self._ret(p, 8); return
                if a == NOOP:
                    self._ret(0); return
            return cb
        for a, n in ((CREATE, 'create'), (READ, 'read'), (SEEK, 'seek'),
                     (NEW, 'new'), (FREE, 'free'), (ALLOC, 'alloc'), (NOOP, 'noop')):
            u.hook_add(UC_HOOK_CODE, hc(a, 4, None, n), begin=a, end=a)
        # patch engine-internal allocators to our bump allocator
        u.mem_write(0x5cb540, struct.pack('<I', CREATE))
        u.mem_write(0x5cb544, struct.pack('<I', READ))
        u.mem_write(0x5cb548, struct.pack('<I', SEEK))
        self.patches = {
            0x4607f4: (lambda: self.alloc(self.arg(0))),
            0x46044a: (lambda: self.alloc(self.arg(0))),
            0x460455: (lambda: 0),
            0x4618a5: (lambda: 0),
            0x439a40: (lambda: 0),
            0x4398b0: None,   # handled by ALLOC magic
        }
        # NOTE: 0x46044a is cdecl; its call site merges stack cleanup into a
        # later `add esp,0x10`, so the emulated callee must NOT pop the arg.
        self.pops = {}

        def mkhook(f):
            def cb(uc, a, sz, ud):
                self._ret(f(), self.pops.get(a, 0))
            return cb
        for addr in (0x4607f4, 0x46044a, 0x460455, 0x4618a5, 0x439a40):
            u.hook_add(UC_HOOK_CODE, mkhook(self.patches[addr]), begin=addr, end=addr)
        # redirect 0x4398b0 -> ALLOC magic by rewriting its entry
        u.mem_write(0x4398b0, b'\xe9' + struct.pack('<i', ALLOC - (0x4398b0 + 5)))

    def setup_object(self, ident=b'eden_en'):
        """Hand-build a ZArchiveFileReader3 instance (enough for OpenVolume)."""
        obj = self.alloc(0x270)
        u = self.uc
        u.mem_write(obj, struct.pack('<I', 0x553054))       # vtable
        # 10 volume handles = 0
        # +0x1D0/+0x1D4/+0x1EC/+0x1F4 = 0 (already zero)
        u.mem_write(obj + 0x220, struct.pack('<I', 0x20))   # header size
        # std::string at +0x1FC (SSO empty)
        u.mem_write(obj + 0x20C, struct.pack('<II', 0, 0xF))
        u.mem_write(obj + 0x1FC, b'\0')
        u.mem_write(obj + 0x198, b'\0')
        u.mem_write(obj + 0x1A8, struct.pack('<II', 0, 0xF))
        self.obj = obj
        return obj

    def open_volume(self, name, vol=0, watch=0x523596):
        u = self.uc
        obj = self.obj
        esp = STACK + STACK_SZ - 0x1000
        u.mem_write(esp, struct.pack('<III', 0xdeadbeef, obj, vol))
        u.reg_write(UC_X86_REG_ESP, esp)
        u.reg_write(UC_X86_REG_ECX, obj)
        u.reg_write(UC_X86_REG_EDX, self.strbuf(name))
        u.reg_write(UC_X86_REG_EIP, 0x5232d0)
        self.watch = watch
        self.watch_hit = False

        def wcb(uc, a, sz, ud):
            if a == self.watch:
                self.watch_hit = True
                uc.emu_stop()
        u.hook_add(UC_HOOK_CODE, wcb, begin=watch, end=watch)
        try:
            u.emu_start(0x5232d0, 0x5232d0 + 0x2000, count=20_000_000)
        except UcError as e:
            print('  emu error:', e, 'eip=%08x' % u.reg_read(UC_X86_REG_EIP))
        return self.watch_hit

    def strbuf(self, b):
        p = self.alloc(len(b) + 1)
        self.uc.mem_write(p, b + b'\0')
        return p

    def dump(self, p, n):
        return bytes(self.uc.mem_read(p, n))

def main():
    emu = Emu('samples/pm/eden_pm_en.exe')
    base = '/mnt/f/gal/【PC硬盘】【官中】eden'
    pm = base + '/eden PLUS MOSAIC'
    emu.basedir = base
    emu.setup_object()
    print('object @%08x' % emu.obj)
    ok = emu.open_volume(b'st', 0)
    print('watch hit:', ok)
    for l in emu.log[:20]:
        print(' ', l)
    if ok:
        # at 0x523596: esi = buffer, [esi] = count
        u = emu.uc
        esi = u.reg_read(UC_X86_REG_ESI)
        print('esi=%08x' % esi)
        data = emu.dump(esi, 64)
        print('buffer:', data.hex())
        print('count =', struct.unpack_from('<I', data, 0)[0])
    print('log tail:', emu.log[-5:])


if __name__ == '__main__':
    main()
