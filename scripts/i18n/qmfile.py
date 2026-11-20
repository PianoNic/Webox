"""Reader for Qt .qm translation files (Qt5/Qt6 format). Shared by the other tools in this directory.

Sections: 0x42 hashes, 0x69 messages, 0x2f contexts, 0x88 numerus rules, 0x96 dependencies, 0xa7 language.
Message records: tag byte + payload; 1=End, 3=Translation (u32 len + UTF-16BE, 0xFFFFFFFF = null),
6=SourceText, 7=Context, 8=Comment (u32 len + UTF-8 bytes).
"""
import os
import struct

MAGIC = bytes.fromhex('3cb86418caef9c95cd211cbf60a1bddd')

# Default locations of the Webex language packs. FULL_DIR is where the installed Windows client keeps them; MIN_DIR
# (the smaller packs) and QML_DIR (the UI files for the developer index) are local folders passed with
# --min-dir / --qml-dir or the WEBOX_QM_MIN_DIR / WEBOX_QML_DIR environment variables.
FULL_DIR = os.environ.get('WEBOX_QM_FULL_DIR') or os.path.join(os.environ.get('LOCALAPPDATA', ''), 'CiscoSparkLauncher', 'dynamic_dependencies', 'translations')
MIN_DIR = os.environ.get('WEBOX_QM_MIN_DIR', '')
QML_DIR = os.environ.get('WEBOX_QML_DIR', '')


def load(path):
    """-> (language, numerus_rules_bytes, [ {ctx,src,cmt,tr:[str|None]} ])"""
    d = open(path, 'rb').read()
    if d[:16] != MAGIC:
        raise ValueError('not a .qm file: ' + path)
    p = 16
    sec = {}
    while p < len(d):
        t = d[p]
        n = struct.unpack('>I', d[p + 1:p + 5])[0]
        sec[t] = d[p + 5:p + 5 + n]
        p += 5 + n
    m = sec[0x69]
    p = 0
    msgs = []
    cur = dict(tr=[], src='', ctx='', cmt='')
    while p < len(m):
        t = m[p]
        p += 1
        if t == 1:
            msgs.append(cur)
            cur = dict(tr=[], src='', ctx='', cmt='')
            continue
        n = struct.unpack('>I', m[p:p + 4])[0]
        p += 4
        if t == 3:
            if n == 0xFFFFFFFF:
                cur['tr'].append(None)
            else:
                cur['tr'].append(m[p:p + n].decode('utf-16-be'))
                p += n
        elif t in (6, 7, 8):
            cur[{6: 'src', 7: 'ctx', 8: 'cmt'}[t]] = m[p:p + n].decode('utf-8', 'replace')
            p += n
        else:
            raise ValueError('unknown message tag %d at %d' % (t, p))
    return sec.get(0xA7, b'').decode(), sec.get(0x88, b''), msgs
