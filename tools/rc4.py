def rc4(key, n):
    S=list(range(256)); j=0
    for i in range(256):
        j=(j+S[i]+key[i%len(key)])&0xff
        S[i],S[j]=S[j],S[i]
    out=bytearray(); i=j=0
    for _ in range(n):
        i=(i+1)&0xff; j=(j+S[i])&0xff
        S[i],S[j]=S[j],S[i]
        out.append(S[(S[i]+S[j])&0xff])
    return bytes(out)
