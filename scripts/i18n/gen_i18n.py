#!/usr/bin/env python
"""Dev-time generator for the embedded localisation packs and the developer index.

  python -I scripts/i18n/gen_i18n.py [--full-dir DIR] [--min-dir DIR] [--qml-dir DIR]

Writes (relative to the repo root):
  assets/cisco/i18n/<locale>.bin.z     compact zlib pack read by src/assets/i18n/   (de = FULL pack, others = launcher 'min' packs)
  assets/cisco/i18n/index_en_de.tsv    context <TAB> English <TAB> German <TAB> QML files  (grep this when converting UI strings)

Compact format (before zlib): b'WXI1', lang str, rules bytes, u32 nctx, ctx strs..., u32 nentries,
entries (ctx_idx varint, src str, nforms varint, forms str...), strings = varint length + UTF-8.
Entries whose translation is identical to the source are dropped (a miss falls back to the source) unless
another context translates the same source differently, or the entry is numerus.
"""
import argparse, glob, os, re, sys, zlib, collections
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import qmfile

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

def varint(n):
    out = bytearray()
    while True:
        b = n & 0x7F
        n >>= 7
        if n:
            out.append(b | 0x80)
        else:
            out.append(b)
            return bytes(out)

def s(x):
    b = x.encode('utf-8')
    return varint(len(b)) + b

def pack(lang, rules, msgs, keep_identical=False):
    # first message per (ctx, src) wins; disambiguation comments are ignored (almost always empty)
    seen = {}
    for m in msgs:
        seen.setdefault((m['ctx'], m['src']), m)
    differing = collections.defaultdict(bool)
    for (c, src), m in seen.items():
        if m['tr'] and m['tr'][0] and m['tr'][0] != src:
            differing[src] = True
    entries = []
    for (c, src), m in sorted(seen.items()):
        tr = [t or '' for t in m['tr']]
        if not any(tr):
            continue
        if not keep_identical and len(tr) == 1 and tr[0] == src and not differing[src]:
            continue
        entries.append((c, src, tr))
    ctxs = sorted({e[0] for e in entries})
    idx = {c: i for i, c in enumerate(ctxs)}
    out = bytearray(b'WXI1')
    out += s(lang) + varint(len(rules)) + rules + varint(len(ctxs))
    for c in ctxs:
        out += s(c)
    out += varint(len(entries))
    for c, src, tr in entries:
        out += varint(idx[c]) + s(src) + varint(len(tr))
        for t in tr:
            out += s(t)
    return zlib.compress(bytes(out), 9), len(entries)

def esc(x):
    return x.replace('\\', '\\\\').replace('\t', '\\t').replace('\n', '\\n').replace('\r', '')

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--full-dir', default=qmfile.FULL_DIR)
    ap.add_argument('--min-dir', default=qmfile.MIN_DIR)
    ap.add_argument('--qml-dir', default=qmfile.QML_DIR)
    a = ap.parse_args()
    outdir = os.path.join(ROOT, 'assets', 'cisco', 'i18n')
    os.makedirs(outdir, exist_ok=True)
    total = 0
    de = None
    # 1. full packs (de, en) from the install dir; en keeps only numerus entries (plural forms).
    for f in sorted(glob.glob(os.path.join(a.full_dir, 'spark-windows-client.*.qm')) + glob.glob(os.path.join(a.min_dir, 'spark-windows-client.*.qm'))):
        loc = f.split('.')[-2]
        if loc == 'en':   # English = the source language (identity); the en pack has no plural forms either
            continue
        lang, rules, msgs = qmfile.load(f)
        data, n = pack(loc, rules, msgs)
        open(os.path.join(outdir, loc + '.bin.z'), 'wb').write(data)
        print('%-6s full %6d entries %8d bytes' % (loc, n, len(data)))
        total += len(data)
        if loc == 'de':
            de = msgs
    # 2. min packs (other 34 locales). Skip those that have a full pack already.
    for f in sorted(glob.glob(os.path.join(a.min_dir, 'spark-windows-client-min.*.qm'))):
        loc = f.split('.')[-2]
        if os.path.exists(os.path.join(outdir, loc + '.bin.z')):
            continue
        if loc == 'en':
            continue
        lang, rules, msgs = qmfile.load(f)
        data, n = pack(loc, rules, msgs)
        open(os.path.join(outdir, loc + '.bin.z'), 'wb').write(data)
        print('%-6s min  %6d entries %8d bytes' % (loc, n, len(data)))
        total += len(data)
    print('total embedded packs: %d bytes' % total)
    # 3. developer index: every source used by the UI files in --qml-dir + the QObject (shared) context.
    if de is None:
        print('no German full pack found; skipping index')
        return
    by = {(m['ctx'], m['src']): m for m in de}
    by_src = collections.defaultdict(list)
    for m in de:
        by_src[m['src']].append(m)
    used = collections.defaultdict(set)   # (ctx, src) -> qml files
    lit = r'"((?:[^"\\]|\\.)*)"|\'((?:[^\'\\]|\\.)*)\''
    def unq(g):
        t = g[0] if g[0] is not None and g[0] != '' else g[1]
        return re.sub(r'\\(.)', lambda m: {'n': '\n', 't': '\t'}.get(m.group(1), m.group(1)), t or '')
    for f in glob.glob(os.path.join(a.qml_dir, '**', '*.qml'), recursive=True):
        base = os.path.splitext(os.path.basename(f))[0]
        txt = open(f, encoding='utf-8', errors='replace').read()
        for m in re.finditer(r'qsTr\(\s*(?:%s)' % lit, txt):
            src = unq(m.groups())
            ctx = base if (base, src) in by else None
            if ctx is None:
                cands = by_src.get(src)
                ctx = cands[0]['ctx'] if cands else base   # unresolved -> still listed with empty German
            used[(ctx, src)].add(base)
        for m in re.finditer(r'qsTranslate\(\s*(?:"([^"]*)"|\'([^\']*)\')\s*,\s*(?:%s)' % lit, txt):
            ctx = m.group(1) or m.group(2)
            src = unq(m.groups()[2:])
            used[(ctx, src)].add(base)
    rows = {}
    for (ctx, src), files in used.items():
        m = by.get((ctx, src))
        rows[(ctx, src)] = (ctx, src, ' || '.join(t for t in m['tr'] if t) if m else '', ','.join(sorted(files)[:3]))
    for m in de:
        if m['ctx'] == 'QObject' and (m['ctx'], m['src']) not in rows:
            rows[(m['ctx'], m['src'])] = (m['ctx'], m['src'], ' || '.join(t for t in m['tr'] if t), '')
    path = os.path.join(outdir, 'index_en_de.tsv')
    with open(path, 'w', encoding='utf-8', newline='\n') as fh:
        fh.write('# context\tenglish\tgerman\tqml_files   (\\n,\\t escaped; QObject = shared JS/C++ strings)\n')
        for k in sorted(rows):
            fh.write('\t'.join(esc(x) for x in rows[k]) + '\n')
    print('index: %d rows, %d bytes (%d qsTr sources used by QML)' % (len(rows), os.path.getsize(path), len(used)))

if __name__ == '__main__':
    main()

