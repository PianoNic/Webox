//! The query language of docs/data-api.md section 3.1, shared by plugins (`data.query` / `data.subscribe`) and, later, theme
//! bindings. A query is parsed once into a tree, compiled once against the model (field names become field indices, relations
//! are resolved, `$params` become lookups) and then evaluated against the store as often as needed.
//!
//! ```text
//! query   := part (";" part)*
//! part    := from [where expr] [search value in path ("," path)*] [group by path] [sort key ("," key)*] [limit n] [offset n]
//! from    := collection | collection "(" value ")"            messages($space)
//! key     := ["-"] path                                         "-" = descending
//! expr    := expr or expr | expr and expr | not expr | "(" expr ")" | value [op value]
//! op      := == | != | < | <= | > | >= | in | ~ | contains | starts_with
//! value   := path | 'text' | "text" | number | true | false | null | $param | @me[.path] | [value, ...] | fn(expr, ...)
//! ```
//!
//! A query may also be JSON: `{"from": "spaces", "where": "unread", "search": {"value": "$q", "in": ["title"]},
//! "group": "kind", "sort": ["-last_activity"], "limit": 50, "offset": 0}` (every key but `from` optional).
use super::model::{self, Kind};
use super::store::{Id, Store};
use serde_json::{Map, Value};
use std::borrow::Cow;
use std::cmp::Ordering;

/// Most items one part of a query may produce.
pub const LIMIT_MAX: usize = 5000;
/// Items a query produces when it names no limit.
pub const LIMIT_DEFAULT: usize = 500;
/// Longest accepted query text.
pub const TEXT_MAX: usize = 4096;

/// A parse or compile error: what is wrong and where (1-based column; 0 = no position, e.g. a JSON query key).
#[derive(Clone, Debug, PartialEq)]
pub struct Error { pub col: usize, pub msg: String }
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.col > 0 { write!(f, "column {}: {}", self.col, self.msg) } else { f.write_str(&self.msg) }
    }
}
fn err<T>(col: usize, msg: impl Into<String>) -> Result<T, Error> { Err(Error { col, msg: msg.into() }) }

// ---------------------------------------------------------------------------------------------------------------- lexer

#[derive(Clone, Debug, PartialEq)]
enum Tok { Ident(String), Str(String), Num(f64), Param(String), Me, LParen, RParen, LBrack, RBrack, Comma, Semi, Dot, Minus, Plus, Op(&'static str), End }

fn describe(t: &Tok) -> String {
    match t {
        Tok::Ident(s) => format!("'{s}'"), Tok::Str(s) => format!("the text '{s}'"), Tok::Num(n) => format!("the number {n}"),
        Tok::Param(p) => format!("${p}"), Tok::Me => "@me".into(), Tok::LParen => "'('".into(), Tok::RParen => "')'".into(),
        Tok::LBrack => "'['".into(), Tok::RBrack => "']'".into(), Tok::Comma => "','".into(), Tok::Semi => "';'".into(),
        Tok::Dot => "'.'".into(), Tok::Minus => "'-'".into(), Tok::Plus => "'+'".into(), Tok::Op(o) => format!("'{o}'"), Tok::End => "the end of the query".into(),
    }
}

fn lex(src: &str) -> Result<Vec<(Tok, usize)>, Error> {
    let cs: Vec<char> = src.chars().collect();
    let (mut i, mut out) = (0, vec![]);
    let word = |c: char| c.is_alphanumeric() || c == '_';
    while i < cs.len() {
        let c = cs[i];
        let col = i + 1;
        if c.is_whitespace() { i += 1; continue; }
        let two: String = cs[i..(i + 2).min(cs.len())].iter().collect();
        let t = match c {
            '(' => Tok::LParen, ')' => Tok::RParen, '[' => Tok::LBrack, ']' => Tok::RBrack, ',' => Tok::Comma, ';' => Tok::Semi,
            '.' if !cs.get(i + 1).is_some_and(char::is_ascii_digit) => Tok::Dot,
            '~' => Tok::Op("~"),
            '=' | '!' | '<' | '>' => {
                let op = match two.as_str() { "==" => "==", "!=" => "!=", "<=" => "<=", ">=" => ">=", _ => match c { '<' => "<", '>' => ">",
                    '=' => return err(col, "'=' compares nothing: write '=='"), _ => return err(col, "'!' is not an operator: write 'not' or '!='") } };
                i += op.len();
                out.push((Tok::Op(op), col));
                continue;
            }
            '\'' | '"' => {
                let mut s = String::new();
                let mut j = i + 1;
                loop {
                    match cs.get(j) {
                        None => return err(col, "text is not closed: a quote is missing"),
                        Some('\\') if cs.get(j + 1).is_some() => { s.push(cs[j + 1]); j += 2; }
                        Some(&q) if q == c => break,
                        Some(&x) => { s.push(x); j += 1; }
                    }
                }
                i = j + 1;
                out.push((Tok::Str(s), col));
                continue;
            }
            '$' => {
                let mut j = i + 1;
                while j < cs.len() && (word(cs[j]) || (cs[j] == '.' && cs.get(j + 1).is_some_and(|c| word(*c)))) { j += 1; }
                if j == i + 1 { return err(col, "'$' must be followed by a parameter name, like $q"); }
                out.push((Tok::Param(cs[i + 1..j].iter().collect()), col));
                i = j;
                continue;
            }
            '@' => {
                let w: String = cs[i + 1..].iter().take_while(|c| word(**c)).collect();
                if w != "me" { return err(col, "the only '@' name is @me"); }
                i += 3;
                out.push((Tok::Me, col));
                continue;
            }
            '-' => Tok::Minus,
            '+' => Tok::Plus,
            c if c.is_ascii_digit() || c == '.' => {
                let mut j = i + 1;
                while j < cs.len() && (cs[j].is_ascii_digit() || cs[j] == '.') { j += 1; }
                let s: String = cs[i..j].iter().collect();
                let n: f64 = s.parse().map_err(|_| Error { col, msg: format!("'{s}' is not a number") })?;
                i = j;
                out.push((Tok::Num(n), col));
                continue;
            }
            c if word(c) => {
                let mut j = i;
                while j < cs.len() && word(cs[j]) { j += 1; }
                let w: String = cs[i..j].iter().collect();
                i = j;
                out.push((Tok::Ident(w), col));
                continue;
            }
            _ => return err(col, format!("unexpected character '{c}'")),
        };
        i += 1;
        out.push((t, col));
    }
    out.push((Tok::End, cs.len() + 1));
    Ok(out)
}

// ---------------------------------------------------------------------------------------------------------------- syntax

/// A path as written: names and the column of the first.
#[derive(Clone, Debug, PartialEq)]
pub struct PathSrc { pub names: Vec<String>, pub col: usize }

#[derive(Clone, Debug, PartialEq)]
pub enum Ast {
    Or(Box<Ast>, Box<Ast>), And(Box<Ast>, Box<Ast>), Not(Box<Ast>),
    Cmp(&'static str, Box<Ast>, Box<Ast>, usize),
    Path(PathSrc), Lit(Value), Param(String), Me(PathSrc), List(Vec<Ast>), Call(String, Vec<Ast>, usize),
}

/// One part of a query as written.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct PartSrc {
    pub from: String, pub from_col: usize, pub arg: Option<Ast>,
    pub filter: Option<Ast>, pub search: Option<(Ast, Vec<PathSrc>)>, pub group: Option<PathSrc>,
    pub sort: Vec<(PathSrc, bool)>, pub limit: Option<usize>, pub offset: usize,
}

struct Parser { t: Vec<(Tok, usize)>, i: usize }

const KEYWORDS: &[&str] = &["where", "search", "in", "group", "by", "sort", "limit", "offset", "and", "or", "not", "contains", "starts_with", "true", "false", "null"];

impl Parser {
    fn peek(&self) -> &Tok { &self.t[self.i].0 }
    fn col(&self) -> usize { self.t[self.i].1 }
    fn next(&mut self) -> (Tok, usize) { let t = self.t[self.i].clone(); if self.i + 1 < self.t.len() { self.i += 1; } t }
    fn is_kw(&self, k: &str) -> bool { matches!(self.peek(), Tok::Ident(w) if w == k) }
    fn eat_kw(&mut self, k: &str) -> bool { if self.is_kw(k) { self.next(); true } else { false } }
    fn expect(&mut self, t: Tok, what: &str) -> Result<(), Error> {
        if *self.peek() == t { self.next(); Ok(()) } else { err(self.col(), format!("expected {what}, found {}", describe(self.peek()))) }
    }
    fn number(&mut self, what: &str) -> Result<usize, Error> {
        match self.next() {
            (Tok::Num(n), _) if n >= 0.0 && n.fract() == 0.0 => Ok(n as usize),
            (t, c) => err(c, format!("{what} needs a whole number, found {}", describe(&t))),
        }
    }
    fn path(&mut self) -> Result<PathSrc, Error> {
        let col = self.col();
        let mut names = vec![];
        loop {
            match self.next() {
                (Tok::Ident(w), _) if !KEYWORDS.contains(&w.as_str()) => names.push(w),
                (t, c) => return err(c, format!("expected a field name, found {}", describe(&t))),
            }
            if *self.peek() != Tok::Dot { break; }
            self.next();
        }
        Ok(PathSrc { names, col })
    }

    fn part(&mut self) -> Result<PartSrc, Error> {
        let mut p = PartSrc { from_col: self.col(), ..Default::default() };
        p.from = match self.next() {
            (Tok::Ident(w), _) => w,
            (t, c) => return err(c, format!("a query starts with a collection such as 'people' or 'spaces', found {}", describe(&t))),
        };
        if *self.peek() == Tok::LParen {
            self.next();
            p.arg = Some(self.value()?);
            self.expect(Tok::RParen, "')' after the collection's argument")?;
        }
        let mut seen: Vec<&str> = vec![];
        loop {
            let col = self.col();
            let kw = match self.peek() { Tok::Ident(w) => w.clone(), Tok::Semi | Tok::End => break, t => return err(col, format!("expected where, search, group by, sort, limit or offset, found {}", describe(t))) };
            let k: &'static str = match kw.as_str() { "where" => "where", "search" => "search", "group" => "group", "sort" => "sort", "limit" => "limit", "offset" => "offset",
                _ => return err(col, format!("expected where, search, group by, sort, limit or offset, found '{kw}'")) };
            if seen.contains(&k) { return err(col, format!("'{k}' appears twice in one query")); }
            seen.push(k);
            self.next();
            match k {
                "where" => p.filter = Some(self.expr()?),
                "search" => {
                    let v = self.value()?;
                    if !self.eat_kw("in") { return err(self.col(), format!("expected 'in' and the fields to search, found {}", describe(self.peek()))); }
                    let mut fields = vec![self.path()?];
                    while *self.peek() == Tok::Comma { self.next(); fields.push(self.path()?); }
                    p.search = Some((v, fields));
                }
                "group" => { if !self.eat_kw("by") { return err(self.col(), "expected 'by' after 'group'"); } p.group = Some(self.path()?); }
                "sort" => loop {
                    let desc = if *self.peek() == Tok::Minus { self.next(); true } else { false };
                    p.sort.push((self.path()?, desc));
                    if *self.peek() != Tok::Comma { break; }
                    self.next();
                },
                "limit" => p.limit = Some(self.number("limit")?),
                _ => p.offset = self.number("offset")?,
            }
        }
        Ok(p)
    }

    fn expr(&mut self) -> Result<Ast, Error> {
        let mut l = self.and()?;
        while self.eat_kw("or") { l = Ast::Or(Box::new(l), Box::new(self.and()?)); }
        Ok(l)
    }
    fn and(&mut self) -> Result<Ast, Error> {
        let mut l = self.unary()?;
        while self.eat_kw("and") { l = Ast::And(Box::new(l), Box::new(self.unary()?)); }
        Ok(l)
    }
    fn unary(&mut self) -> Result<Ast, Error> {
        if self.eat_kw("not") { return Ok(Ast::Not(Box::new(self.unary()?))); }
        if *self.peek() == Tok::LParen {
            self.next();
            let e = self.expr()?;
            self.expect(Tok::RParen, "')'")?;
            return Ok(e);
        }
        self.cmp()
    }
    fn cmp(&mut self) -> Result<Ast, Error> {
        let l = self.value()?;
        let col = self.col();
        let op = match self.peek() {
            Tok::Op(o) => *o,
            Tok::Ident(w) if w == "in" => "in", Tok::Ident(w) if w == "contains" => "contains", Tok::Ident(w) if w == "starts_with" => "starts_with",
            _ => return Ok(l),
        };
        self.next();
        if matches!(self.peek(), Tok::End | Tok::Semi | Tok::RParen | Tok::Comma) { return err(self.col(), format!("expected a value after '{op}'")); }
        let r = self.value()?;
        Ok(Ast::Cmp(op, Box::new(l), Box::new(r), col))
    }
    /// A value with `+` / `-` between numbers (`now() - days(7)`).
    fn value(&mut self) -> Result<Ast, Error> {
        let mut l = self.atom()?;
        loop {
            let (op, col) = match self.peek() { Tok::Plus => ("+", self.col()), Tok::Minus => ("-", self.col()), _ => return Ok(l) };
            self.next();
            l = Ast::Cmp(op, Box::new(l), Box::new(self.atom()?), col);
        }
    }
    fn atom(&mut self) -> Result<Ast, Error> {
        let col = self.col();
        match self.peek().clone() {
            Tok::Str(s) => { self.next(); Ok(Ast::Lit(Value::String(s))) }
            Tok::Num(n) => { self.next(); Ok(Ast::Lit(num(n))) }
            Tok::Minus => match self.t.get(self.i + 1) { Some((Tok::Num(n), _)) => { let n = *n; self.next(); self.next(); Ok(Ast::Lit(num(-n))) } _ => err(col, "'-' here must be followed by a number") },
            Tok::Param(p) => { self.next(); Ok(Ast::Param(p)) }
            Tok::Me => {
                self.next();
                if *self.peek() == Tok::Dot { self.next(); Ok(Ast::Me(self.path()?)) } else { Ok(Ast::Me(PathSrc { names: vec![], col })) }
            }
            Tok::LBrack => {
                self.next();
                let mut items = vec![];
                if *self.peek() != Tok::RBrack {
                    loop { items.push(self.value()?); if *self.peek() != Tok::Comma { break; } self.next(); }
                }
                self.expect(Tok::RBrack, "']' to close the list")?;
                Ok(Ast::List(items))
            }
            Tok::Ident(w) => match w.as_str() {
                "true" => { self.next(); Ok(Ast::Lit(Value::Bool(true))) }
                "false" => { self.next(); Ok(Ast::Lit(Value::Bool(false))) }
                "null" => { self.next(); Ok(Ast::Lit(Value::Null)) }
                _ if matches!(self.t.get(self.i + 1), Some((Tok::LParen, _))) => {
                    self.next();
                    self.next();
                    let mut args = vec![];
                    if *self.peek() != Tok::RParen {
                        loop { args.push(self.expr()?); if *self.peek() != Tok::Comma { break; } self.next(); }
                    }
                    self.expect(Tok::RParen, &format!("')' to close {w}(")) ?;
                    Ok(Ast::Call(w, args, col))
                }
                _ if KEYWORDS.contains(&w.as_str()) => err(col, format!("expected a value, found the keyword '{w}'")),
                _ => Ok(Ast::Path(self.path()?)),
            },
            t => err(col, format!("expected a value, found {}", describe(&t))),
        }
    }
}

fn num(n: f64) -> Value {
    if n.fract() == 0.0 && n.abs() < 9.0e15 { Value::from(n as i64) } else { serde_json::Number::from_f64(n).map_or(Value::Null, Value::Number) }
}

/// Parses the text form into its parts.
pub fn parse(src: &str) -> Result<Vec<PartSrc>, Error> {
    if src.len() > TEXT_MAX { return err(0, format!("the query is longer than {TEXT_MAX} characters")); }
    if src.trim().is_empty() { return err(0, "the query is empty"); }
    let mut p = Parser { t: lex(src)?, i: 0 };
    let mut parts = vec![p.part()?];
    while *p.peek() == Tok::Semi { p.next(); parts.push(p.part()?); }
    if *p.peek() != Tok::End { return err(p.col(), format!("unexpected {}", describe(p.peek()))); }
    Ok(parts)
}

/// Parses an expression alone (a JSON query's `where`).
fn parse_expr(src: &str) -> Result<Ast, Error> {
    let mut p = Parser { t: lex(src)?, i: 0 };
    let e = p.expr()?;
    if *p.peek() != Tok::End { return err(p.col(), format!("unexpected {}", describe(p.peek()))); }
    Ok(e)
}
fn parse_path(src: &str) -> Result<PathSrc, Error> {
    let mut p = Parser { t: lex(src)?, i: 0 };
    let x = p.path()?;
    if *p.peek() != Tok::End { return err(p.col(), format!("unexpected {}", describe(p.peek()))); }
    Ok(x)
}

/// Parses the JSON form (`{"from", "where", "search", "group", "sort", "limit", "offset"}`), or a list of them for `;`.
pub fn parse_json(v: &Value) -> Result<Vec<PartSrc>, Error> {
    if let Some(list) = v.as_array() {
        if list.is_empty() { return err(0, "the query list is empty"); }
        return list.iter().map(|x| parse_json(x).map(|mut p| p.remove(0))).collect();
    }
    let Some(o) = v.as_object() else { return err(0, "a query is text or an object with \"from\"") };
    for k in o.keys() {
        if !["from", "where", "search", "group", "sort", "limit", "offset"].contains(&k.as_str()) {
            return err(0, format!("unknown query key \"{k}\" (known: from, where, search, group, sort, limit, offset)"));
        }
    }
    let ctx = |k: &str, e: Error| Error { col: 0, msg: format!("\"{k}\": {e}") };
    let Some(from) = o.get("from").and_then(Value::as_str) else { return err(0, "\"from\" is missing: name a collection such as \"spaces\"") };
    let mut p = parse(from).map_err(|e| ctx("from", e))?.remove(0);
    if p.filter.is_some() || p.search.is_some() || !p.sort.is_empty() { return err(0, "\"from\" names only the collection; put the rest in its own keys"); }
    if let Some(w) = o.get("where") {
        let Some(w) = w.as_str() else { return err(0, "\"where\" must be text") };
        p.filter = Some(parse_expr(w).map_err(|e| ctx("where", e))?);
    }
    if let Some(s) = o.get("search") {
        let (val, fields) = match s {
            Value::String(t) => {
                let mut ps = Parser { t: lex(t).map_err(|e| ctx("search", e))?, i: 0 };
                let v = ps.value().map_err(|e| ctx("search", e))?;
                if !ps.eat_kw("in") { return err(0, "\"search\": expected '$param in field, ...'"); }
                let mut f = vec![ps.path().map_err(|e| ctx("search", e))?];
                while *ps.peek() == Tok::Comma { ps.next(); f.push(ps.path().map_err(|e| ctx("search", e))?); }
                (v, f)
            }
            Value::Object(so) => {
                let v = match so.get("value") {
                    Some(Value::String(t)) if t.starts_with('$') => Ast::Param(t[1..].to_string()),
                    Some(Value::String(t)) => Ast::Lit(Value::String(t.clone())),
                    _ => return err(0, "\"search\": \"value\" must be text or \"$param\""),
                };
                let f = so.get("in").and_then(Value::as_array).ok_or_else(|| Error { col: 0, msg: "\"search\": \"in\" must list fields".into() })?;
                let f: Result<Vec<PathSrc>, Error> = f.iter().map(|x| x.as_str().ok_or(Error { col: 0, msg: "\"search\": fields are text".into() }).and_then(|t| parse_path(t).map_err(|e| ctx("search", e)))).collect();
                (v, f?)
            }
            _ => return err(0, "\"search\" must be text or {\"value\", \"in\"}"),
        };
        if fields.is_empty() { return err(0, "\"search\": name at least one field"); }
        p.search = Some((val, fields));
    }
    if let Some(g) = o.get("group") { p.group = Some(parse_path(g.as_str().unwrap_or("")).map_err(|e| ctx("group", e))?); }
    if let Some(s) = o.get("sort") {
        let keys: Vec<&str> = match s { Value::String(t) => t.split(',').collect(), Value::Array(a) => a.iter().filter_map(Value::as_str).collect(), _ => vec![] };
        if keys.is_empty() { return err(0, "\"sort\" must be text or a list of fields"); }
        for k in keys {
            let k = k.trim();
            let (desc, k) = k.strip_prefix('-').map_or((false, k), |r| (true, r));
            p.sort.push((parse_path(k).map_err(|e| ctx("sort", e))?, desc));
        }
    }
    if let Some(l) = o.get("limit") { p.limit = Some(l.as_u64().ok_or_else(|| Error { col: 0, msg: "\"limit\" must be a whole number".into() })? as usize); }
    if let Some(l) = o.get("offset") { p.offset = l.as_u64().ok_or_else(|| Error { col: 0, msg: "\"offset\" must be a whole number".into() })? as usize; }
    Ok(vec![p])
}

// ---------------------------------------------------------------------------------------------------------------- compiled

/// One step of a compiled path.
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    /// Field `idx` of an entity of `kind`; `rel` = the kind its id points to (`list` = a list of ids).
    Field { kind: Kind, idx: usize, rel: Option<Kind>, list: bool },
    /// A key of an object value (or of each object of a list).
    Key(String),
}
#[derive(Clone, Debug, PartialEq)]
pub struct Path { pub steps: Vec<Step> }
impl Path {
    /// The kind a path's value refers to (a relation), if any.
    fn target(&self) -> Option<(Kind, bool)> {
        match self.steps.last() { Some(Step::Field { rel: Some(k), list, .. }) => Some((*k, *list)), _ => None }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Func { Len, Any, All, Now, Minutes, Hours, Days, Lower }

#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Or(Box<Node>, Box<Node>), And(Box<Node>, Box<Node>), Not(Box<Node>),
    Cmp(&'static str, Box<Node>, Box<Node>),
    Path(Path), Lit(Value), Param(String), List(Vec<Node>),
    /// The Me entity: its person id, or a path on it.
    Me(Option<Path>),
    /// A built-in state key (`$space.selected`, `$nav.page`, `$team.selected`): a field of the App entity unless a param of
    /// that name is given.
    State(String, usize),
    Call(Func, Vec<Node>),
    /// The current item (inside any / all over a list of plain values).
    It,
}

/// Where paths are resolved: an entity kind, or the objects of a list (keys unchecked).
#[derive(Clone, Copy)]
enum Scope { Entity(Kind), Object }

/// A compiled part.
#[derive(Clone, Debug)]
pub struct Part {
    pub kind: Kind,
    pub collection: &'static str,
    pub arg: Option<Node>,
    pub filter: Option<Node>,
    /// Hidden spaces / non-contacts are left out first.
    pub implicit: Option<(usize, bool)>,
    pub search: Option<(Node, Vec<Path>)>,
    pub group: Option<Path>,
    pub sort: Vec<(Path, bool)>,
    pub limit: usize,
    pub offset: usize,
}

/// A compiled query, and which fields of which kinds it reads (a change elsewhere cannot change its result).
#[derive(Clone, Debug)]
pub struct Query { pub parts: Vec<Part>, pub reads: [u64; 14] }

struct Compiler { reads: [u64; 14], hidden_named: bool, in_item: bool }

/// The closest of `known` to `w` (a typo), for "did you mean".
fn suggest<'a>(w: &str, known: impl Iterator<Item = &'a str>) -> String {
    let d = |a: &str, b: &str| {
        let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
        let mut prev: Vec<usize> = (0..=b.len()).collect();
        let mut prev2: Vec<usize> = prev.clone();
        for i in 1..=a.len() {
            let mut cur = vec![i; b.len() + 1];
            for j in 1..=b.len() {
                cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + usize::from(a[i - 1] != b[j - 1]));
                // a swap of two neighbours counts as one edit
                if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] { cur[j] = cur[j].min(prev2[j - 2] + 1); }
            }
            prev2 = std::mem::replace(&mut prev, cur);
        }
        prev[b.len()]
    };
    known.map(|k| (d(w, k), k)).filter(|(n, _)| *n <= 2.max(w.len() / 3)).min().map_or(String::new(), |(_, k)| format!(" (did you mean '{k}'?)"))
}

impl Compiler {
    fn path(&mut self, scope: Scope, p: &PathSrc, sortable: bool) -> Result<Path, Error> {
        let mut steps = vec![];
        let mut cur = scope;
        for (i, name) in p.names.iter().enumerate() {
            match cur {
                Scope::Object => { steps.push(Step::Key(name.clone())); }
                Scope::Entity(k) => {
                    let Some(idx) = k.field(name) else {
                        let known: Vec<&str> = k.def().fields.iter().map(|f| f.name).collect();
                        return err(p.col, format!("'{name}' is not a field of {}{}; fields: {}", k.name(), suggest(name, known.iter().copied()), known.join(", ")));
                    };
                    let f = &k.def().fields[idx];
                    if f.volatile && sortable { return err(p.col, format!("{}.{name} is read when asked: it can be selected, not filtered or sorted", k.name())); }
                    self.reads[k.idx()] |= 1u64 << idx;
                    if k == Kind::Space && name == "hidden" && i == 0 { self.hidden_named = true; }
                    let list = f.ty.starts_with('[');
                    steps.push(Step::Field { kind: k, idx, rel: f.rel, list });
                    cur = match f.rel {
                        Some(t) => { self.reads[t.idx()] |= 1; Scope::Entity(t) }
                        None if f.ty == "object" && i + 1 < p.names.len() => {
                            if !f.keys.is_empty() && !f.keys.contains(&p.names[i + 1].as_str()) {
                                return err(p.col, format!("'{}' is not a key of {}.{name}; keys: {}", p.names[i + 1], k.name(), f.keys.join(", ")));
                            }
                            Scope::Object
                        }
                        None if i + 1 < p.names.len() && f.ty.starts_with('[') => Scope::Object,
                        None if i + 1 < p.names.len() => return err(p.col, format!("{}.{name} is a {}, it has no '{}'", k.name(), f.ty, p.names[i + 1])),
                        None => cur,
                    };
                }
            }
        }
        Ok(Path { steps })
    }

    fn node(&mut self, scope: Scope, a: &Ast) -> Result<Node, Error> {
        Ok(match a {
            Ast::Or(l, r) => Node::Or(Box::new(self.node(scope, l)?), Box::new(self.node(scope, r)?)),
            Ast::And(l, r) => Node::And(Box::new(self.node(scope, l)?), Box::new(self.node(scope, r)?)),
            Ast::Not(x) => Node::Not(Box::new(self.node(scope, x)?)),
            Ast::Cmp(op, l, r, _) => Node::Cmp(op, Box::new(self.node(scope, l)?), Box::new(self.node(scope, r)?)),
            Ast::Lit(v) => Node::Lit(v.clone()),
            Ast::List(l) => Node::List(l.iter().map(|x| self.node(scope, x)).collect::<Result<_, _>>()?),
            // inside any / all: `it` is the item, `it.x` (or just `x`) a field or key of it
            Ast::Path(p) if self.in_item && p.names.len() == 1 && p.names[0] == "it" => Node::It,
            Ast::Path(p) if self.in_item && p.names[0] == "it" => Node::Path(self.path(scope, &PathSrc { names: p.names[1..].to_vec(), col: p.col }, true)?),
            Ast::Path(p) => Node::Path(self.path(scope, p, true)?),
            Ast::Param(p) => match p.as_str() {
                "space.selected" => { self.reads[Kind::App.idx()] |= 1u64 << Kind::App.f("selected"); Node::State(p.clone(), Kind::App.f("selected")) }
                "nav.page" => { self.reads[Kind::App.idx()] |= 1u64 << Kind::App.f("page"); Node::State(p.clone(), Kind::App.f("page")) }
                "team.selected" => { self.reads[Kind::App.idx()] |= 1u64 << Kind::App.f("team"); Node::State(p.clone(), Kind::App.f("team")) }
                _ => Node::Param(p.clone()),
            },
            Ast::Me(p) => {
                self.reads[Kind::Me.idx()] |= 1u64 << Kind::Me.f("person");
                if p.names.is_empty() { Node::Me(None) } else { Node::Me(Some(self.path(Scope::Entity(Kind::Me), p, true)?)) }
            }
            Ast::Call(name, args, col) => {
                let (f, n) = match name.as_str() {
                    "len" | "count" => (Func::Len, 1), "any" => (Func::Any, 2), "all" => (Func::All, 2), "now" => (Func::Now, 0),
                    "minutes" => (Func::Minutes, 1), "hours" => (Func::Hours, 1), "days" => (Func::Days, 1), "lower" => (Func::Lower, 1),
                    _ => return err(*col, format!("unknown function '{name}'{}; functions: len, count, any, all, now, minutes, hours, days, lower",
                        suggest(name, ["len", "count", "any", "all", "now", "minutes", "hours", "days", "lower"].into_iter()))),
                };
                if args.len() != n { return err(*col, format!("{name}() takes {n} argument{}, got {}", if n == 1 { "" } else { "s" }, args.len())); }
                if matches!(f, Func::Any | Func::All) {
                    let list = self.node(scope, &args[0])?;
                    let inner = match &list { Node::Path(p) => match p.target() { Some((k, true)) => Scope::Entity(k), _ => Scope::Object }, _ => Scope::Object };
                    let was = std::mem::replace(&mut self.in_item, true);
                    let e = self.node(inner, &args[1]);
                    self.in_item = was;
                    Node::Call(f, vec![list, e?])
                } else {
                    Node::Call(f, args.iter().map(|x| self.node(scope, x)).collect::<Result<_, _>>()?)
                }
            }
        })
    }
}

/// Compiles parsed parts against the model.
pub fn compile(parts: &[PartSrc]) -> Result<Query, Error> {
    let mut c = Compiler { reads: [0; 14], hidden_named: false, in_item: false };
    let mut out = vec![];
    for p in parts {
        let Some(col) = model::collection(&p.from) else {
            return err(p.from_col, format!("unknown collection '{}'{}; collections: {}", p.from, suggest(&p.from, model::COLLECTIONS.iter().map(|c| c.name)),
                model::COLLECTIONS.iter().map(|c| c.name).collect::<Vec<_>>().join(", ")));
        };
        match (col.arg, &p.arg) {
            (Some(a), None) => return err(p.from_col, format!("'{}' needs an argument: {}({})", col.name, col.name, a)),
            (None, Some(_)) => return err(p.from_col, format!("'{}' takes no argument", col.name)),
            _ => {}
        }
        let kind = col.kind;
        let scope = Scope::Entity(kind);
        c.reads[kind.idx()] |= 1;
        c.hidden_named = false;
        let arg = p.arg.as_ref().map(|a| c.node(scope, a)).transpose()?;
        let filter = p.filter.as_ref().map(|a| c.node(scope, a)).transpose()?;
        let search = match &p.search {
            Some((v, fields)) => Some((c.node(scope, v)?, fields.iter().map(|f| c.path(scope, f, true)).collect::<Result<Vec<_>, _>>()?)),
            None => None,
        };
        let group = p.group.as_ref().map(|g| c.path(scope, g, true)).transpose()?;
        let sort = p.sort.iter().map(|(s, d)| c.path(scope, s, true).map(|x| (x, *d))).collect::<Result<Vec<_>, _>>()?;
        let implicit = match col.name {
            "spaces" if !c.hidden_named => { c.reads[kind.idx()] |= 1u64 << Kind::Space.f("hidden"); Some((Kind::Space.f("hidden"), false)) }
            "contacts" => { c.reads[kind.idx()] |= 1u64 << Kind::Person.f("contact"); Some((Kind::Person.f("contact"), true)) }
            "messages" | "thread" => { c.reads[kind.idx()] |= 1u64 << Kind::Message.f("space") | 1u64 << Kind::Message.f("parent"); None }
            _ => None,
        };
        let limit = p.limit.unwrap_or(LIMIT_DEFAULT);
        if limit > LIMIT_MAX { return err(0, format!("limit {limit} is above the maximum of {LIMIT_MAX}")); }
        out.push(Part { kind, collection: col.name, arg, filter, implicit, search, group, sort, limit, offset: p.offset });
    }
    Ok(Query { parts: out, reads: c.reads })
}

/// Compiles a selected field (`presence.state`) of `kind`; volatile fields are allowed here. Returns the path and whether
/// its first field is volatile.
pub fn compile_field(kind: Kind, text: &str) -> Result<(Path, bool), Error> {
    let p = parse_path(text)?;
    let mut c = Compiler { reads: [0; 14], hidden_named: false, in_item: false };
    let path = c.path(Scope::Entity(kind), &p, false)?;
    let volatile = matches!(path.steps.first(), Some(Step::Field { kind, idx, .. }) if kind.def().fields[*idx].volatile);
    Ok((path, volatile))
}

/// Parses and compiles text or JSON.
pub fn compile_value(q: &Value) -> Result<Query, Error> {
    match q { Value::String(s) => compile(&parse(s)?), Value::Object(_) | Value::Array(_) => compile(&parse_json(q)?), _ => err(0, "\"query\" must be text or an object") }
}

// ---------------------------------------------------------------------------------------------------------------- evaluation

/// What a query is evaluated with.
pub struct Env<'a> { pub store: &'a Store, pub params: &'a Map<String, Value>, pub now_ms: i64 }

/// The item a path starts from.
#[derive(Clone, Copy)]
enum At<'a> { Entity(Kind, &'a str), Obj(&'a Value) }

impl<'a> Env<'a> {
    /// Resolves a path from entity `id` of `kind`.
    pub fn resolve(&self, kind: Kind, id: &str, p: &Path) -> Cow<'a, Value> { self.walk(At::Entity(kind, id), &p.steps) }

    fn walk(&self, at: At<'_>, steps: &[Step]) -> Cow<'a, Value> {
        let Some((s, rest)) = steps.split_first() else { return Cow::Owned(Value::Null) };
        let v: Cow<'a, Value> = match (s, at) {
            (Step::Field { kind, idx, .. }, At::Entity(k, id)) if *kind == k => match self.store.value(k, id, *idx) { Some(v) => Cow::Borrowed(v), None => return Cow::Owned(Value::Null) },
            (Step::Key(key), At::Obj(o)) => match o { Value::Object(m) => Cow::Owned(m.get(key).cloned().unwrap_or(Value::Null)),
                Value::Array(a) => Cow::Owned(Value::Array(a.iter().map(|x| x.get(key).cloned().unwrap_or(Value::Null)).collect())), _ => Cow::Owned(Value::Null) },
            _ => return Cow::Owned(Value::Null),
        };
        if rest.is_empty() { return v; }
        match s {
            Step::Field { rel: Some(t), list: false, .. } => match v.as_str() { Some(id) => self.walk(At::Entity(*t, id), rest), None => Cow::Owned(Value::Null) },
            Step::Field { rel: Some(t), list: true, .. } => Cow::Owned(Value::Array(v.as_array().into_iter().flatten().filter_map(Value::as_str)
                .map(|id| self.walk(At::Entity(*t, id), rest).into_owned()).collect())),
            _ => Cow::Owned(self.walk(At::Obj(&v), rest).into_owned()),
        }
    }

    fn me_person(&self) -> Value { self.store.value(Kind::Me, "me", Kind::Me.f("person")).cloned().unwrap_or(Value::Null) }

    fn eval(&self, n: &Node, at: At<'_>, it: Option<&Value>) -> Value {
        match n {
            Node::Lit(v) => v.clone(),
            Node::It => it.cloned().unwrap_or(Value::Null),
            Node::Path(p) => self.walk(at, &p.steps).into_owned(),
            Node::Param(p) => self.params.get(p).cloned().unwrap_or(Value::Null),
            Node::State(name, f) => self.params.get(name).cloned().unwrap_or_else(|| self.store.value(Kind::App, "app", *f).cloned().unwrap_or(Value::Null)),
            Node::Me(None) => self.me_person(),
            Node::Me(Some(p)) => self.walk(At::Entity(Kind::Me, "me"), &p.steps).into_owned(),
            Node::List(l) => Value::Array(l.iter().map(|x| self.eval(x, at, it)).collect()),
            Node::Or(l, r) => Value::Bool(truthy(&self.eval(l, at, it)) || truthy(&self.eval(r, at, it))),
            Node::And(l, r) => Value::Bool(truthy(&self.eval(l, at, it)) && truthy(&self.eval(r, at, it))),
            Node::Not(x) => Value::Bool(!truthy(&self.eval(x, at, it))),
            Node::Cmp(op @ ("+" | "-"), l, r) => match (self.eval(l, at, it).as_f64(), self.eval(r, at, it).as_f64()) {
                (Some(a), Some(b)) => num(if *op == "+" { a + b } else { a - b }),
                _ => Value::Null, // numbers only: no text is ever joined
            },
            Node::Cmp(op, l, r) => Value::Bool(compare(op, &self.eval(l, at, it), &self.eval(r, at, it))),
            Node::Call(f, args) => match f {
                Func::Now => Value::from(self.now_ms),
                Func::Minutes | Func::Hours | Func::Days => {
                    let n = self.eval(&args[0], at, it).as_f64().unwrap_or(0.0);
                    num(n * match f { Func::Minutes => 60_000.0, Func::Hours => 3_600_000.0, _ => 86_400_000.0 })
                }
                Func::Lower => self.eval(&args[0], at, it).as_str().map_or(Value::Null, |s| Value::String(s.to_lowercase())),
                Func::Len => match self.eval(&args[0], at, it) { Value::Array(a) => Value::from(a.len()), Value::String(s) => Value::from(s.chars().count()), Value::Null => Value::from(0), _ => Value::from(1) },
                Func::Any | Func::All => {
                    let list = self.eval(&args[0], at, it);
                    let items = list.as_array().cloned().unwrap_or_default();
                    let target = match &args[0] { Node::Path(p) => p.target().filter(|t| t.1).map(|t| t.0), _ => None };
                    let test = |x: &Value| match (target, x.as_str()) {
                        (Some(k), Some(id)) => truthy(&self.eval(&args[1], At::Entity(k, id), Some(x))),
                        _ => truthy(&self.eval(&args[1], At::Obj(x), Some(x))),
                    };
                    Value::Bool(if *f == Func::Any { items.iter().any(test) } else { items.iter().all(test) })
                }
            },
        }
    }

    /// Whether entity `id` passes `filter`.
    pub fn test(&self, kind: Kind, id: &str, filter: &Node) -> bool { truthy(&self.eval(filter, At::Entity(kind, id), None)) }
    pub fn value_of(&self, n: &Node) -> Value { self.eval(n, At::Obj(&Value::Null), None) }

    /// The ids `part` produces, in order, and the total before limit / offset, with group boundaries (key, count).
    pub fn run(&self, part: &Part) -> (Vec<Id>, usize, Vec<(Value, usize)>) {
        let t = self.store.table(part.kind);
        // the candidates
        let mut ids: Vec<&Id> = match part.collection {
            "messages" => { let s = part.arg.as_ref().map(|a| self.value_of(a)); s.as_ref().and_then(Value::as_str).map_or(vec![], |s| self.store.group(Kind::Message, s).collect()) }
            "thread" => {
                let root = part.arg.as_ref().map(|a| self.value_of(a));
                let root = root.as_ref().and_then(Value::as_str).unwrap_or("");
                let space = self.store.value(Kind::Message, root, Kind::Message.f("space")).and_then(Value::as_str).unwrap_or("");
                let pf = Kind::Message.f("parent");
                self.store.group(Kind::Message, space).filter(|id| &***id == root || self.store.value(Kind::Message, id, pf).and_then(Value::as_str) == Some(root)).collect()
            }
            _ => t.items.keys().collect(),
        };
        if let Some((f, want)) = part.implicit {
            ids.retain(|id| self.store.value(part.kind, id, f).and_then(Value::as_bool).unwrap_or(false) == want);
        }
        if let Some(fl) = &part.filter { ids.retain(|id| self.test(part.kind, id, fl)); }
        // search: matches only, prefix matches first
        let mut rank: std::collections::HashMap<&Id, u8> = Default::default();
        if let Some((v, fields)) = &part.search {
            let q = fold(self.value_of(v).as_str().unwrap_or("").trim());
            if !q.is_empty() {
                ids.retain(|id| {
                    let mut best = None;
                    for f in fields {
                        let val = self.resolve(part.kind, id, f);
                        for s in strings(&val) {
                            let h = fold(s);
                            if h.starts_with(&q) || h.split(|c: char| !c.is_alphanumeric()).any(|w| w.starts_with(&q)) { best = Some(0); break; }
                            if h.contains(&q) { best = best.or(Some(1)); }
                        }
                        if best == Some(0) { break; }
                    }
                    if let Some(b) = best { rank.insert(id, b); }
                    best.is_some()
                });
            }
        }
        // order: group key, search rank, sort keys, then the source order and the id (stable)
        let key = |id: &Id, p: &Path| self.resolve(part.kind, id, p).into_owned();
        let mut rows: Vec<(&Id, Option<Value>, Vec<Value>, u32)> = ids.into_iter().map(|id| {
            (id, part.group.as_ref().map(|g| key(id, g)), part.sort.iter().map(|(p, _)| key(id, p)).collect(), t.items.get(id).map_or(0, |i| i.order))
        }).collect();
        rows.sort_by(|a, b| {
            let g = match (&a.1, &b.1) { (Some(x), Some(y)) => order(x, y, false), _ => Ordering::Equal };
            g.then_with(|| rank.get(a.0).cmp(&rank.get(b.0)))
                .then_with(|| a.2.iter().zip(&b.2).zip(&part.sort).map(|((x, y), (_, d))| order(x, y, *d)).find(|o| *o != Ordering::Equal).unwrap_or(Ordering::Equal))
                .then_with(|| a.3.cmp(&b.3)).then_with(|| a.0.cmp(b.0))
        });
        let total = rows.len();
        let page: Vec<_> = rows.into_iter().skip(part.offset).take(part.limit).collect();
        let mut groups: Vec<(Value, usize)> = vec![];
        if part.group.is_some() {
            for r in &page {
                let k = r.1.clone().unwrap_or(Value::Null);
                match groups.last_mut() { Some((g, n)) if *g == k => *n += 1, _ => groups.push((k, 1)) }
            }
        }
        (page.into_iter().map(|r| r.0.clone()).collect(), total, groups)
    }
}

/// The strings in a value (a list's items too).
fn strings(v: &Value) -> Vec<&str> {
    match v { Value::String(s) => vec![s.as_str()], Value::Array(a) => a.iter().flat_map(strings).collect(), _ => vec![] }
}

pub fn truthy(v: &Value) -> bool {
    match v { Value::Null => false, Value::Bool(b) => *b, Value::Number(n) => n.as_f64() != Some(0.0), Value::String(s) => !s.is_empty(), Value::Array(a) => !a.is_empty(), Value::Object(o) => !o.is_empty() }
}

/// Lower case without accents, for `~` and `search` ("Zoë" ~ "zoe").
pub fn fold(s: &str) -> String {
    s.chars().flat_map(char::to_lowercase).map(|c| match c {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą' => 'a', 'ç' | 'ć' | 'č' => 'c', 'ď' => 'd',
        'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ė' | 'ę' | 'ě' => 'e', 'ì' | 'í' | 'î' | 'ï' | 'ī' | 'į' | 'ı' => 'i', 'ñ' | 'ń' | 'ň' => 'n',
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' | 'ő' => 'o', 'ř' => 'r', 'ś' | 'š' | 'ş' => 's', 'ť' | 'ţ' => 't',
        'ù' | 'ú' | 'û' | 'ü' | 'ū' | 'ů' | 'ű' | 'ų' => 'u', 'ý' | 'ÿ' => 'y', 'ź' | 'ż' | 'ž' => 'z', 'ł' => 'l', 'ß' => 's', c => c,
    }).collect()
}

fn eq(a: &Value, b: &Value) -> bool {
    match (a, b) { (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(), _ => a == b }
}

fn compare(op: &str, l: &Value, r: &Value) -> bool {
    match op {
        "==" => eq(l, r),
        "!=" => !eq(l, r),
        "<" | "<=" | ">" | ">=" => {
            if l.is_null() || r.is_null() { return false; }
            let o = match (l, r) {
                (Value::Number(x), Value::Number(y)) => x.as_f64().partial_cmp(&y.as_f64()),
                (Value::String(x), Value::String(y)) => Some(x.cmp(y)),
                (Value::Bool(x), Value::Bool(y)) => Some(x.cmp(y)),
                _ => None,
            };
            o.is_some_and(|o| match op { "<" => o.is_lt(), "<=" => o.is_le(), ">" => o.is_gt(), _ => o.is_ge() })
        }
        "in" => match r { Value::Array(a) => a.iter().any(|x| eq(l, x)), Value::String(s) => l.as_str().is_some_and(|x| s.contains(x)), _ => false },
        "contains" => match l { Value::Array(a) => a.iter().any(|x| eq(x, r)), Value::String(s) => r.as_str().is_some_and(|x| s.contains(x)), _ => false },
        "starts_with" => matches!((l, r), (Value::String(s), Value::String(p)) if s.starts_with(p.as_str())),
        "~" => {
            let Some(q) = r.as_str() else { return false };
            let q = fold(q);
            strings(l).iter().any(|s| fold(s).contains(&q))
        }
        _ => false,
    }
}

/// The sort order: absent values last in both directions, then numbers, texts (case and accents folded first) and others.
pub fn order(a: &Value, b: &Value, desc: bool) -> Ordering {
    let rank = |v: &Value| match v { Value::Null => 9, Value::Bool(_) => 0, Value::Number(_) => 1, Value::String(_) => 2, _ => 3 };
    match (a.is_null(), b.is_null()) { (true, true) => return Ordering::Equal, (true, false) => return Ordering::Greater, (false, true) => return Ordering::Less, _ => {} }
    let o = match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64().partial_cmp(&y.as_f64()).unwrap_or(Ordering::Equal),
        (Value::String(x), Value::String(y)) => fold(x).cmp(&fold(y)).then_with(|| x.cmp(y)),
        (Value::Bool(x), Value::Bool(y)) => x.cmp(y),
        _ => rank(a).cmp(&rank(b)),
    };
    if desc { o.reverse() } else { o }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(q: &str) -> String { match parse(q).and_then(|p| compile(&p)) { Ok(_) => "ok".into(), Err(e) => e.to_string() } }

    #[test]
    fn valid_queries_parse_and_compile() {
        for q in [
            "people",
            "people where presence.online and name ~ $q sort name limit 500",
            "spaces where kind == 'direct' or (kind == 'group' and team == null) sort -last_activity limit 200",
            "spaces where team == $team sort -favorite, title",
            "people where presence.in_call sort presence.state, name",
            "teams sort name; spaces where favorite and team == null sort title",
            "calls where state != 'ended' limit 1",
            "contacts search $q in name, presence.text sort name",
            "messages($space) where author == @me and at > now() - days(7) sort -at",
            "thread('m1')",
            "spaces where len(members) >= 3 and any(members, presence.online)",
            "spaces where not hidden and $space.selected != id",
            "call_history where missed and at > now() - hours(24) sort -at limit 20 offset 5",
            "people where lower(name) starts_with 'a' and name in ['Anna', \"Ben\"]",
            "messages($s) where any(reactions, it.mine) group by kind",
            "spaces where meeting.active == true",
            "me", "app", "settings where id == 'theme'",
            "people where @me.person.name != name",
            "spaces where -1 < 0",
        ] { assert_eq!(msg(q), "ok", "{q}"); }
    }

    #[test]
    fn invalid_queries_say_what_and_where() {
        let cases = [
            ("", "the query is empty"),
            ("peeple", "column 1: unknown collection 'peeple' (did you mean 'people'?)"),
            ("people where nmae == 'x'", "column 14: 'nmae' is not a field of person (did you mean 'name'?)"),
            ("people where name = 'x'", "column 19: '=' compares nothing: write '=='"),
            ("people where name == ", "column 22: expected a value after '=='"),
            ("people where name == 'x", "column 22: text is not closed: a quote is missing"),
            ("people sort", "column 12: expected a field name, found the end of the query"),
            ("people limit -3", "column 14: limit needs a whole number, found '-'"),
            ("people limit 99999", "limit 99999 is above the maximum of 5000"),
            ("people where name == 'a' where name == 'b'", "column 26: 'where' appears twice in one query"),
            ("messages", "column 1: 'messages' needs an argument: messages(space id)"),
            ("people(1)", "column 1: 'people' takes no argument"),
            ("people where frob(name)", "column 14: unknown function 'frob'; functions:"),
            ("people where len(name, 1)", "column 14: len() takes 1 argument, got 2"),
            ("spaces where meeting.nope", "column 14: 'nope' is not a key of space.meeting; keys: active, locus"),
            ("spaces where title.x", "column 14: space.title is a string, it has no 'x'"),
            ("calls sort stats", "column 12: call.stats is read when asked: it can be selected, not filtered or sorted"),
            ("people where @you", "column 14: the only '@' name is @me"),
            ("people where name == 'a' sort", "column 30: expected a field name, found the end of the query"),
            ("people frob", "column 8: expected where, search, group by, sort, limit or offset, found 'frob'"),
            ("people search $q name", "column 18: expected 'in' and the fields to search, found 'name'"),
            ("people where (name == 'a'", "column 26: expected ')', found the end of the query"),
            ("people where name == 'a' )", "column 26: expected where, search, group by, sort, limit or offset, found ')'"),
        ];
        for (q, want) in cases {
            let got = msg(q);
            assert!(got.starts_with(want), "{q}\n got: {got}\nwant: {want}");
        }
    }

    #[test]
    fn json_queries_compile_like_text() {
        let j = serde_json::json!({ "from": "spaces", "where": "kind == 'direct' or team == null", "search": { "value": "$q", "in": ["title"] },
            "sort": ["-last_activity", "title"], "limit": 200 });
        let q = compile_value(&j).unwrap();
        let t = compile(&parse("spaces where kind == 'direct' or team == null search $q in title sort -last_activity, title limit 200").unwrap()).unwrap();
        assert_eq!(format!("{:?}", q.parts[0].filter), format!("{:?}", t.parts[0].filter));
        assert_eq!(q.parts[0].sort, t.parts[0].sort);
        assert_eq!(q.parts[0].limit, 200);
        assert!(compile_value(&serde_json::json!({ "from": "spaces", "wehre": "x" })).unwrap_err().msg.contains("unknown query key \"wehre\""));
        assert!(compile_value(&serde_json::json!({ "where": "x" })).unwrap_err().msg.contains("\"from\" is missing"));
        assert!(compile_value(&serde_json::json!({ "from": "spaces", "where": "nope" })).unwrap_err().msg.contains("'nope' is not a field of space"));
        assert!(compile_value(&serde_json::json!(5)).is_err());
    }

    #[test]
    fn reads_name_the_fields_a_query_depends_on() {
        let q = compile(&parse("people where presence.online sort name").unwrap()).unwrap();
        assert_ne!(q.reads[Kind::Person.idx()] & 1u64 << Kind::Person.f("name"), 0);
        assert_ne!(q.reads[Kind::Person.idx()] & 1u64 << Kind::Person.f("presence"), 0);
        assert_ne!(q.reads[Kind::Presence.idx()] & 1u64 << Kind::Presence.f("online"), 0);
        assert_eq!(q.reads[Kind::Person.idx()] & 1u64 << Kind::Person.f("email"), 0);
        assert_eq!(q.reads[Kind::Space.idx()], 0);
    }

    #[test]
    fn fold_and_order() {
        assert_eq!(fold("Zoë Ångström"), "zoe angstrom");
        assert!(compare("~", &Value::from("Zoë"), &Value::from("ZOE")));
        assert!(compare("in", &Value::from(2), &serde_json::json!([1, 2.0])));
        assert_eq!(order(&Value::Null, &Value::from(1), false), Ordering::Greater);
        assert_eq!(order(&Value::Null, &Value::from(1), true), Ordering::Greater, "absent stays last when descending");
        assert_eq!(order(&Value::from("b"), &Value::from("A"), false), Ordering::Greater);
    }
}
