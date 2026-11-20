#!/usr/bin/env python
"""Dev tool: look up translation contexts and German strings in the Webex language packs.

  python -I scripts/i18n/find_qm_context.py "Add Giphy"        # exact English source
  python -I scripts/i18n/find_qm_context.py -s "giphy"         # substring (case-insensitive) in English OR translation
  python -I scripts/i18n/find_qm_context.py -c ConversationHelper   # list one context
  options: --lang de (default; any pack file spark-windows-client[-min].<lang>.qm), --pack <file.qm>

Output: context <TAB> English source <TAB> translation(s) (numerus forms joined with ' || ').
"""
import argparse, os, sys, io
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import qmfile

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('text', nargs='?')
    ap.add_argument('-s', '--substring', action='store_true')
    ap.add_argument('-c', '--context')
    ap.add_argument('--lang', default='de')
    ap.add_argument('--pack')
    a = ap.parse_args()
    pack = a.pack
    if not pack:
        for cand in (os.path.join(qmfile.FULL_DIR, 'spark-windows-client.%s.qm' % a.lang),
                     os.path.join(qmfile.MIN_DIR, 'spark-windows-client.%s.qm' % a.lang),
                     os.path.join(qmfile.MIN_DIR, 'spark-windows-client-min.%s.qm' % a.lang)):
            if os.path.exists(cand):
                pack = cand
                break
    if not pack:
        sys.exit('no pack for language ' + a.lang)
    sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding='utf-8')
    lang, rules, msgs = qmfile.load(pack)
    needle = (a.text or '').lower()
    n = 0
    for m in sorted(msgs, key=lambda m: (m['ctx'], m['src'])):
        tr = [t for t in m['tr'] if t]
        if a.context and m['ctx'] != a.context:
            continue
        if a.text:
            if a.substring:
                if needle not in m['src'].lower() and not any(needle in t.lower() for t in tr):
                    continue
            elif m['src'] != a.text:
                continue
        elif not a.context:
            sys.exit('give an English text, -s substring or -c context')
        print('%s\t%s\t%s' % (m['ctx'], m['src'].replace('\n', '\\n'), ' || '.join(tr).replace('\n', '\\n')))
        n += 1
    print('# %d match(es) in %s' % (n, os.path.basename(pack)), file=sys.stderr)

if __name__ == '__main__':
    main()
