//! Qt .qm reader and the in-memory translation catalog.
use super::*;

// Qt .qm reader.

pub(super) const QM_MAGIC: [u8; 16] = [
    0x3c, 0xb8, 0x64, 0x18, 0xca, 0xef, 0x9c, 0x95, 0xcd, 0x21, 0x1c, 0xbf, 0x60, 0xa1, 0xbd, 0xdd,
];

/// One message of a `.qm` file. `translations` has one entry per plural form (None = null string).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QmMessage {
    pub context: String,
    pub source: String,
    pub comment: String,
    pub translations: Vec<Option<String>>,
}

/// A parsed `.qm` file.
#[derive(Debug, Clone, Default)]
pub struct Qm {
    pub language: String,
    /// Qt numerus rule bytecode (section 0x88).
    pub numerus_rules: Vec<u8>,
    pub messages: Vec<QmMessage>,
}

pub(super) fn be32(d: &[u8], p: usize) -> Result<u32, String> {
    d.get(p..p + 4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]])).ok_or_else(|| "truncated .qm".to_string())
}

/// Parse a Qt `.qm` file (Qt 5/6). Sections other than language, numerus rules and messages are skipped
/// (hash/contexts tables are only lookup accelerators; messages are read sequentially).
pub fn parse_qm(d: &[u8]) -> Result<Qm, String> {
    if d.len() < 16 || d[..16] != QM_MAGIC {
        return Err("bad .qm magic".into());
    }
    let mut qm = Qm::default();
    let mut p = 16;
    let mut messages: &[u8] = &[];
    while p < d.len() {
        let tag = d[p];
        let len = be32(d, p + 1)? as usize;
        let body = d.get(p + 5..p + 5 + len).ok_or("truncated .qm section")?;
        match tag {
            0x69 => messages = body,
            0x88 => qm.numerus_rules = body.to_vec(),
            0xa7 => qm.language = String::from_utf8_lossy(body).into_owned(),
            _ => {}
        }
        p += 5 + len;
    }
    let m = messages;
    let mut p = 0;
    let mut cur = QmMessage { context: String::new(), source: String::new(), comment: String::new(), translations: vec![] };
    while p < m.len() {
        let tag = m[p];
        p += 1;
        if tag == 1 {
            qm.messages.push(std::mem::replace(
                &mut cur,
                QmMessage { context: String::new(), source: String::new(), comment: String::new(), translations: vec![] },
            ));
            continue;
        }
        let len = be32(m, p)?;
        p += 4;
        match tag {
            3 => {
                if len == 0xffff_ffff {
                    cur.translations.push(None);
                } else {
                    let b = m.get(p..p + len as usize).ok_or("truncated translation")?;
                    let u: Vec<u16> = b.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
                    cur.translations.push(Some(String::from_utf16_lossy(&u)));
                    p += len as usize;
                }
            }
            6 | 7 | 8 => {
                let b = m.get(p..p + len as usize).ok_or("truncated string")?;
                let s = String::from_utf8_lossy(b).into_owned();
                match tag {
                    6 => cur.source = s,
                    7 => cur.context = s,
                    _ => cur.comment = s,
                }
                p += len as usize;
            }
            // Obsolete 16-bit variants (2 = SourceText16, 4 = Context16, 5 = Obsolete1): skip payload.
            2 | 4 | 5 => p += if len == 0xffff_ffff { 0 } else { len as usize },
            t => return Err(format!("unknown message tag {t}")),
        }
    }
    Ok(qm)
}

/// Qt numerus rule interpreter: form index for `n`.
pub fn numerus_index(rules: &[u8], n: i64) -> usize {
    const Q_EQ: u8 = 1;
    const Q_LT: u8 = 2;
    const Q_LEQ: u8 = 3;
    const Q_BETWEEN: u8 = 4;
    const Q_NOT: u8 = 0x08;
    const Q_MOD_10: u8 = 0x10;
    const Q_MOD_100: u8 = 0x20;
    const Q_LEAD_1000: u8 = 0x40;
    const Q_AND: u8 = 0xfd;
    const Q_OR: u8 = 0xfe;
    let n = n.unsigned_abs() as i64;
    if rules.is_empty() {
        return 0;
    }
    let size = rules.len();
    let mut result = 0usize;
    let mut i = 0usize;
    loop {
        let mut or_val = false;
        loop {
            let mut and_val = true;
            loop {
                let Some(&opcode) = rules.get(i) else { return result };
                i += 1;
                let mut left = n;
                if opcode & Q_MOD_10 != 0 {
                    left %= 10;
                } else if opcode & Q_MOD_100 != 0 {
                    left %= 100;
                } else if opcode & Q_LEAD_1000 != 0 {
                    while left >= 1000 {
                        left /= 1000;
                    }
                }
                let right = rules.get(i).copied().unwrap_or(0) as i64;
                i += 1;
                let mut truth = match opcode & 0x07 {
                    Q_EQ => left == right,
                    Q_LT => left < right,
                    Q_LEQ => left <= right,
                    Q_BETWEEN => {
                        let top = rules.get(i).copied().unwrap_or(0) as i64;
                        i += 1;
                        left >= right && left <= top
                    }
                    _ => true,
                };
                if opcode & Q_NOT != 0 {
                    truth = !truth;
                }
                and_val = and_val && truth;
                if i == size || rules[i] != Q_AND {
                    break;
                }
                i += 1;
            }
            or_val = or_val || and_val;
            if i == size || rules[i] != Q_OR {
                break;
            }
            i += 1;
        }
        if or_val {
            return result;
        }
        result += 1;
        if i == size {
            break;
        }
        i += 1; // Q_NEWRULE
    }
    result
}

// Catalog (context -> source -> plural forms).

/// An in-memory translation catalog for one language.
#[derive(Debug, Default)]
pub struct Catalog {
    pub lang: String,
    rules: Vec<u8>,
    by_ctx: HashMap<String, HashMap<String, Vec<String>>>,
    /// Source -> context used for context-less lookups (QObject preferred, else alphabetically first).
    any: HashMap<String, String>,
}

pub(super) fn first_nonempty(forms: &[String]) -> Option<&str> {
    forms.iter().map(|s| s.as_str()).find(|s| !s.is_empty())
}

impl Catalog {
    pub fn len(&self) -> usize {
        self.by_ctx.values().map(|m| m.len()).sum()
    }
    pub fn is_empty(&self) -> bool {
        self.by_ctx.is_empty()
    }

    pub(super) fn insert(&mut self, ctx: &str, src: &str, forms: Vec<String>) {
        if first_nonempty(&forms).is_none() {
            return;
        }
        self.by_ctx.entry(ctx.to_string()).or_default().entry(src.to_string()).or_insert(forms);
    }

    pub(super) fn rebuild_any(&mut self) {
        let mut ctxs: Vec<&String> = self.by_ctx.keys().collect();
        ctxs.sort();
        // QObject first so it wins, then alphabetical.
        ctxs.sort_by_key(|c| c.as_str() != "QObject");
        let mut any = HashMap::new();
        for c in ctxs {
            for s in self.by_ctx[c].keys() {
                any.entry(s.clone()).or_insert_with(|| c.clone());
            }
        }
        self.any = any;
    }

    /// Build from a parsed `.qm`.
    pub fn from_qm(qm: &Qm) -> Catalog {
        let mut c = Catalog { lang: qm.language.clone(), rules: qm.numerus_rules.clone(), ..Default::default() };
        c.merge_qm(qm);
        c
    }

    pub(super) fn merge_qm(&mut self, qm: &Qm) {
        if !qm.numerus_rules.is_empty() {
            self.rules = qm.numerus_rules.clone();
        }
        for m in &qm.messages {
            let forms: Vec<String> = m.translations.iter().map(|t| t.clone().unwrap_or_default()).collect();
            if first_nonempty(&forms).is_none() {
                continue;
            }
            // Overlay semantics: later pack wins.
            self.by_ctx.entry(m.context.clone()).or_default().insert(m.source.clone(), forms);
        }
        self.rebuild_any();
    }

    /// Decode the compact pack written by `scripts/i18n/gen_i18n.py` (zlib of `WXI1` format).
    pub fn from_pack(z: &[u8]) -> Result<Catalog, String> {
        let mut raw = Vec::new();
        flate2::read::ZlibDecoder::new(z).read_to_end(&mut raw).map_err(|e| e.to_string())?;
        let mut r = Reader { d: &raw, p: 0 };
        if r.take(4)? != b"WXI1" {
            return Err("bad pack magic".into());
        }
        let mut c = Catalog { lang: r.string()?, ..Default::default() };
        let nr = r.varint()? as usize;
        c.rules = r.take(nr)?.to_vec();
        let nctx = r.varint()? as usize;
        let mut ctxs = Vec::with_capacity(nctx);
        for _ in 0..nctx {
            ctxs.push(r.string()?);
        }
        let n = r.varint()? as usize;
        for _ in 0..n {
            let ci = r.varint()? as usize;
            let src = r.string()?;
            let nf = r.varint()? as usize;
            let mut forms = Vec::with_capacity(nf);
            for _ in 0..nf {
                forms.push(r.string()?);
            }
            let ctx = ctxs.get(ci).ok_or("bad ctx index")?;
            c.insert(ctx, &src, forms);
        }
        c.rebuild_any();
        Ok(c)
    }

    pub(super) fn forms(&self, ctx: &str, src: &str) -> Option<&Vec<String>> {
        if let Some(f) = self.by_ctx.get(ctx).and_then(|m| m.get(src)) {
            return Some(f);
        }
        let c = self.any.get(src)?;
        self.by_ctx.get(c)?.get(src)
    }

    /// Translation of `src` (first plural form), if any.
    pub fn get(&self, ctx: &str, src: &str) -> Option<&str> {
        first_nonempty(self.forms(ctx, src)?)
    }

    /// Plural form for `n` (falls back to the first non-empty form like Qt does for empty forms).
    pub fn get_n(&self, ctx: &str, src: &str, n: i64) -> Option<&str> {
        let f = self.forms(ctx, src)?;
        let i = numerus_index(&self.rules, n).min(f.len().saturating_sub(1));
        f.get(i).map(|s| s.as_str()).filter(|s| !s.is_empty()).or_else(|| first_nonempty(f))
    }

    /// Exact-context lookup only (no fallback), used by dev tooling and tests.
    pub fn get_exact(&self, ctx: &str, src: &str) -> Option<&str> {
        first_nonempty(self.by_ctx.get(ctx)?.get(src)?)
    }
}

pub(super) struct Reader<'a> {
    d: &'a [u8],
    p: usize,
}
impl<'a> Reader<'a> {
    pub(super) fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let s = self.d.get(self.p..self.p + n).ok_or("truncated pack")?;
        self.p += n;
        Ok(s)
    }
    pub(super) fn varint(&mut self) -> Result<u64, String> {
        let mut v = 0u64;
        let mut sh = 0;
        loop {
            let b = self.take(1)?[0];
            v |= ((b & 0x7f) as u64) << sh;
            if b & 0x80 == 0 {
                return Ok(v);
            }
            sh += 7;
            if sh > 63 {
                return Err("varint overflow".into());
            }
        }
    }
    pub(super) fn string(&mut self) -> Result<String, String> {
        let n = self.varint()? as usize;
        Ok(String::from_utf8_lossy(self.take(n)?).into_owned())
    }
}
