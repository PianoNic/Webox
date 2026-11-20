"""The `*_16` toolbar icons are font glyphs (1024-unit em box, flipped with scale(1,-1), no viewBox), which resvg draws off-canvas.
This writes `<name>_em.svg` copies (same path data) with the em box as viewBox (0 -960 1024 1024), so every glyph keeps its
true size inside the em box. Usage: python -I gen_em_icons.py <repo root>"""
import re, sys
root = sys.argv[1]
for n in ['attachment_16', 'screenshot_16', 'markdown_16', 'emoticons_16', 'mention_16']:
    t = open(f'{root}/assets/icons/{n}.svg', encoding='utf-8').read()
    t = re.sub(r'<\?xml.*?\?>', '', t, flags=re.S)
    t = re.sub(r'<!DOCTYPE.*?>', '', t, flags=re.S)
    inner = re.search(r'<svg\b[^>]*>(.*)</svg>', t, flags=re.S).group(1)
    open(f'{root}/assets/icons/{n}_em.svg', 'w', encoding='utf-8', newline='\n').write(
        f'<svg xmlns="http://www.w3.org/2000/svg" width="1024" height="1024" viewBox="0 -960 1024 1024">{inner}</svg>\n')
