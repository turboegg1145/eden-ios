#!/usr/bin/env python3
"""Minimal Blowfish (ECB/CBC) implemented from the S/P tables embedded in eden.exe.
Used to verify the .paz index encryption hypothesis.
"""
import struct

def load_tables(exe_path):
    d = open(exe_path, 'rb').read()
    off = d.find(struct.pack('<I', 0x243F6A88))
    assert off > 0
    vals = struct.unpack_from('<1042I', d, off)
    P = list(vals[:18])
    S = [list(vals[18+i*256:18+(i+1)*256]) for i in range(4)]
    return P, S

class Blowfish:
    def __init__(self, key, P0, S0):
        self.P = list(P0); self.S = [list(x) for x in S0]
        self._key(key)
    def _F(self, x):
        S = self.S
        a = (x >> 24) & 0xff; b = (x >> 16) & 0xff; c = (x >> 8) & 0xff; d = x & 0xff
        return (((S[0][a] + S[1][b]) & 0xffffffff) ^ S[2][c]) + S[3][d] & 0xffffffff
    def _enc(self, L, R):
        P = self.P; F = self._F
        for i in range(16):
            L ^= P[i]
            R ^= F(L)
            L, R = R, L
        L, R = R, L
        R ^= P[16]
        L ^= P[17]
        return L & 0xffffffff, R & 0xffffffff
    def _dec(self, L, R):
        P = self.P; F = self._F
        for i in range(17, 1, -1):
            L ^= P[i]
            R ^= F(L)
            L, R = R, L
        L, R = R, L
        R ^= P[1]
        L ^= P[0]
        return L & 0xffffffff, R & 0xffffffff
    def _key(self, key):
        k = key
        j = 0
        for i in range(18):
            v = 0
            for _ in range(4):
                v = ((v << 8) | k[j % len(k)]) & 0xffffffff
                j += 1
            self.P[i] ^= v
        L = R = 0
        for i in range(0, 18, 2):
            L, R = self._enc(L, R)
            self.P[i] = L; self.P[i+1] = R
        for s in range(4):
            for i in range(0, 256, 2):
                L, R = self._enc(L, R)
                self.S[s][i] = L; self.S[s][i+1] = R
    def encrypt_block(self, b):
        L, R = struct.unpack('>II', b)
        L, R = self._enc(L, R)
        return struct.pack('>II', L, R)
    def decrypt_block(self, b):
        L, R = struct.unpack('>II', b)
        L, R = self._dec(L, R)
        return struct.pack('>II', L, R)

def ecb_dec(data, key, P, S):
    bf = Blowfish(key, P, S)
    out = b''
    for i in range(0, len(data) - len(data) % 8, 8):
        out += bf.decrypt_block(data[i:i+8])
    return out

def cbc_dec(data, key, iv, P, S):
    bf = Blowfish(key, P, S)
    out = b''; prev = iv
    for i in range(0, len(data) - len(data) % 8, 8):
        blk = data[i:i+8]
        out += bytes(a ^ b for a, b in zip(bf.decrypt_block(blk), prev))
        prev = blk
    return out

if __name__ == '__main__':
    import sys
    exe = sys.argv[1] if len(sys.argv) > 1 else 'samples/main/eden.exe'
    P, S = load_tables(exe)
    print('P0', hex(P[0]), 'S0', hex(S[0][0]))
