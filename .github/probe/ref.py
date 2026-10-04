import sys, struct
from decimal import Decimal, getcontext
from multiprocessing import Pool
getcontext().prec = 70
PI = Decimal("3.14159265358979323846264338327950288419716939937510582097494459230781640628620899")
HALF_PI = PI / 2

def f2d(h): return Decimal(struct.unpack('>d', bytes.fromhex(h))[0])
def d2h(d):
    return struct.pack('>d', float(d)).hex()
def h2f(h): return struct.unpack('>d', bytes.fromhex(h))[0]

def sin_cos(x):
    # x in [0, pi/2+]; Taylor after halving
    n = 0
    while abs(x) > Decimal("0.01"):
        x /= 2; n += 1
    s = x; c = Decimal(1); term_s = x; term_c = Decimal(1); k = 1
    x2 = x * x
    term_c = Decimal(1); term_s = x
    s = x; c = Decimal(1)
    for i in range(1, 25):
        term_c = -term_c * x2 / ((2*i-1)*(2*i))
        term_s = -term_s * x2 / ((2*i)*(2*i+1))
        c += term_c; s += term_s
    for _ in range(n):
        s, c = 2*s*c, c*c - s*s
    return s, c

def tan(x):
    neg = x < 0
    s, c = sin_cos(abs(x))
    t = s / c
    return -t if neg else t

def atan_pos(x):
    # x >= 0
    if x > 1:
        return HALF_PI - atan_pos(1 / x)
    n = 0
    while x > Decimal("0.01"):
        x = x / (1 + (1 + x*x).sqrt()); n += 1
    x2 = x*x; term = x; s = x
    for i in range(1, 30):
        term = -term * x2
        s += term / (2*i+1)
    return s * (2 ** n)

def atan(x):
    return -atan_pos(-x) if x < 0 else atan_pos(x)

def atan2(y, x):
    if x == 0:
        if y == 0: return Decimal(0)
        return HALF_PI if y > 0 else -HALF_PI
    a = atan(y / x)
    if x > 0: return a
    return a + PI if y >= 0 else a - PI

def work(line):
    p = line.split()
    fn = p[0]
    if fn == 'tan':
        r = tan(f2d(p[1])); return fn, p[2], p[3], d2h(r)
    if fn == 'atan':
        r = atan(f2d(p[1])); return fn, p[2], p[3], d2h(r)
    r = atan2(f2d(p[1]), f2d(p[2])); return fn, p[3], p[4], d2h(r)

def ulps(a, b):
    ia = struct.unpack('>q', bytes.fromhex(a))[0]; ib = struct.unpack('>q', bytes.fromhex(b))[0]
    return abs(ia - ib)

if __name__ == '__main__':
    lines = open(sys.argv[1]).read().split('\n')
    lines = [l for l in lines if l]
    seen = set(); uniq = []
    for l in lines:
        if l not in seen: seen.add(l); uniq.append(l)
    with Pool(8) as pool:
        res = pool.map(work, uniq, chunksize=500)
    stats = {}
    for fn, std, lib, ref in res:
        st = stats.setdefault(fn, dict(n=0, std_bad=0, lib_bad=0, std_vs_lib=0, std_max=0, lib_max=0))
        st['n'] += 1
        u1 = ulps(std, ref); u2 = ulps(lib, ref)
        if u1: st['std_bad'] += 1
        if u2: st['lib_bad'] += 1
        if std != lib: st['std_vs_lib'] += 1
        st['std_max'] = max(st['std_max'], u1); st['lib_max'] = max(st['lib_max'], u2)
    for fn, st in stats.items(): print(sys.argv[2] if len(sys.argv)>2 else '', fn, st)
