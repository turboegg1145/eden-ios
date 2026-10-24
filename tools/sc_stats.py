#!/usr/bin/env python3
"""Scan extracted .sc scripts and produce a command/argument census.

Outputs JSON (machine readable) and a Markdown table (for the port plan).
Comments start with ';' (the fan-translation patch uses them heavily).
'.message' takes a free-form text tail, so it is tokenised separately.
"""
import os, re, sys, json, collections

NUM = re.compile(rb'^-?\d+$')


def tokenize(line):
    """Return (cmd, args) where args is a list of byte tokens."""
    parts = line.split()
    cmd = parts[0]
    if cmd == b'.message':
        # .message <id> [voice] [#]text...
        return cmd, parts[1:]
    return cmd, parts[1:]


def shape(tok):
    if NUM.match(tok):
        return 'n'
    if tok in (b'*', b'-'):
        return tok.decode()
    if re.match(rb'^[A-Za-z0-9_\-]+\.(png|ogg|sc|ani|avi|mpg|wav|txt)$', tok, re.I):
        return 'file'
    return 'str'


def scan(dirs):
    stats = {}
    lines_total = 0
    for d in dirs:
        for fn in sorted(os.listdir(d)):
            if not fn.lower().endswith('.sc'):
                continue
            raw = open(os.path.join(d, fn), 'rb').read()
            for ln in raw.split(b'\r\n'):
                ln = ln.strip()
                if not ln or ln.startswith(b';'):
                    continue
                lines_total += 1
                if not ln.startswith(b'.'):
                    stats.setdefault('<bare>', collections.Counter())['bare'] += 1
                    continue
                cmd, args = tokenize(ln)
                key = cmd.decode('latin1')
                s = stats.setdefault(key, collections.Counter())
                s['total'] += 1
                s['argc=%d' % len(args)] += 1
                sh = ' '.join(shape(a) for a in args[:6])
                s['shape:' + sh] += 1
                if 'ex' not in s:
                    s['ex'] = []
                if len(s['ex']) < 4:
                    s['ex'].append(ln.decode('gbk', 'replace')[:160])
    return stats, lines_total


def main():
    dirs = sys.argv[1:] or ['extracted/main_scr', 'extracted/pm_scr']
    stats, n = scan(dirs)
    print('lines', n, 'commands', len(stats))
    out = {}
    for cmd, c in sorted(stats.items(), key=lambda kv: -kv[1]['total']):
        total = c['total']
        argc = {k: v for k, v in c.items() if k.startswith('argc=')}
        shapes = {k[6:]: v for k, v in c.items() if k.startswith('shape:')}
        print('\n%s  total=%d' % (cmd, total))
        print('  argc:', dict(sorted(argc.items())))
        for sh, v in sorted(shapes.items(), key=lambda kv: -kv[1])[:6]:
            print('   %5d  %s' % (v, sh))
        for e in c.get('ex', [])[:2]:
            print('   e.g.', e)
        out[cmd] = {'total': total, 'argc': argc, 'shapes': shapes,
                    'examples': c.get('ex', [])}
    json.dump(out, open('reports/sc_command_census.json', 'w'),
              ensure_ascii=False, indent=1)


if __name__ == '__main__':
    main()
