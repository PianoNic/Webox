"""Builds assets/emoji_shortcodes.tsv: `char<TAB>english name<TAB>source` for every emoji of assets/emoji_data.tsv.

English names come from Unicode CLDR (the `tts` names of common/annotations/en.xml and common/annotationsDerived/en.xml,
Unicode licence); single-code-point emoji CLDR does not name fall back to the Unicode character name.
A local copy of the Webex table can live in assets/cisco/emoji_shortcodes.tsv and wins at build time.

Usage: python -I gen_emoji_shortcodes.py <cldr annotations/en.xml> <cldr annotationsDerived/en.xml> <repo root>"""
import io, sys, unicodedata
import xml.etree.ElementTree as ET

ann, derived, root = sys.argv[1], sys.argv[2], sys.argv[3]
VS16 = '️'
names = {}
for path in (ann, derived):
    for a in ET.parse(path).getroot().iter('annotation'):
        if a.get('type') == 'tts' and a.text:
            names.setdefault(a.get('cp').replace(VS16, ''), a.text.strip())

out = io.open(root + '/assets/emoji_shortcodes.tsv', 'w', encoding='utf-8', newline='\n')
for line in io.open(root + '/assets/emoji_data.tsv', encoding='utf-8'):
    r = line.rstrip('\n').split('\t')
    if len(r) < 4:
        continue
    ch = r[2]
    base = ch.replace(VS16, '')
    en, src = names.get(base), 'cldr'
    if not en and len(base) == 1:
        try:
            en = unicodedata.name(base).lower()
            for suf in (' sign', ' symbol'):
                if en.endswith(suf):
                    en = en[:-len(suf)]
            src = 'unicode'
        except ValueError:
            en = None
    if not en:
        en, src = '', 'none'
    out.write(f'{ch}\t{en}\t{src}\n')
