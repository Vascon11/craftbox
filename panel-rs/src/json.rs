//! JSON com a semântica do JavaScript (JSON.parse / JSON.stringify do Node).
//!
//! Por que não serde_json: o contrato é "a mesma resposta que o server.js", e o
//! server.js tem semântica de JS: todo número é f64 e sai formatado como
//! `Number.prototype.toString` (12 e não 12.0; 1e21 e não 1000000000000000000000),
//! objetos preservam a ordem de inserção (com chaves "índice de array" primeiro,
//! como no V8) e o config.json é regravado com `JSON.stringify(cfg, null, 2)`.
//! Um módulo de ~300 linhas dá isso exato e evita serde + syn na compilação.

use std::fmt::Write as _;

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// `undefined` do JS: some como valor de objeto no stringify e vira `null` em array
    Undef,
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Value>),
    Obj(Map),
}

/// Objeto com ordem de inserção (semântica de objeto JS).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Map(Vec<(String, Value)>);

/// "Array index" do ECMAScript: inteiro canônico em [0, 2^32-2]. O V8 enumera
/// essas chaves primeiro, em ordem numérica, antes das demais.
fn array_index(k: &str) -> Option<u32> {
    if k.is_empty() || k.len() > 10 || !k.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if k.len() > 1 && k.starts_with('0') {
        return None;
    }
    let n: u64 = k.parse().ok()?;
    if n < 4_294_967_295 {
        Some(n as u32)
    } else {
        None
    }
}

impl Map {
    pub fn new() -> Self {
        Map(Vec::new())
    }
    pub fn get(&self, k: &str) -> Option<&Value> {
        self.0.iter().find(|(kk, _)| kk == k).map(|(_, v)| v)
    }
    pub fn get_mut(&mut self, k: &str) -> Option<&mut Value> {
        self.0.iter_mut().find(|(kk, _)| kk == k).map(|(_, v)| v)
    }
    /// Atribuição JS: chave existente mantém a posição, nova vai pro fim.
    pub fn insert(&mut self, k: impl Into<String>, v: Value) {
        let k = k.into();
        if let Some(slot) = self.get_mut(&k) {
            *slot = v;
        } else {
            self.0.push((k, v));
        }
    }
    pub fn remove(&mut self, k: &str) -> Option<Value> {
        let i = self.0.iter().position(|(kk, _)| kk == k)?;
        Some(self.0.remove(i).1)
    }
    pub fn len(&self) -> usize {
        self.0.len()
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    /// Itera na ordem de enumeração do V8 (índices numéricos primeiro).
    pub fn iter(&self) -> impl Iterator<Item = (&String, &Value)> {
        let mut idx: Vec<(u32, &(String, Value))> =
            self.0.iter().filter_map(|e| array_index(&e.0).map(|n| (n, e))).collect();
        idx.sort_by_key(|(n, _)| *n);
        let rest = self.0.iter().filter(|e| array_index(&e.0).is_none());
        idx.into_iter().map(|(_, e)| e).chain(rest).map(|(k, v)| (k, v))
    }
}

impl Value {
    pub fn get(&self, k: &str) -> Option<&Value> {
        match self {
            Value::Obj(m) => m.get(k),
            _ => None,
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Num(n) => Some(*n),
            _ => None,
        }
    }
    pub fn as_obj(&self) -> Option<&Map> {
        match self {
            Value::Obj(m) => Some(m),
            _ => None,
        }
    }
    pub fn as_arr(&self) -> Option<&Vec<Value>> {
        match self {
            Value::Arr(a) => Some(a),
            _ => None,
        }
    }
    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }
    /// `v.k` que pode ser undefined (campo ausente → `Undef`, que some no stringify)
    pub fn fld(&self, k: &str) -> Value {
        self.get(k).cloned().unwrap_or(Value::Undef)
    }
}

impl From<&str> for Value {
    fn from(s: &str) -> Self {
        Value::Str(s.to_string())
    }
}
impl From<String> for Value {
    fn from(s: String) -> Self {
        Value::Str(s)
    }
}
impl From<bool> for Value {
    fn from(b: bool) -> Self {
        Value::Bool(b)
    }
}
impl From<f64> for Value {
    fn from(n: f64) -> Self {
        Value::Num(n)
    }
}
impl From<i64> for Value {
    fn from(n: i64) -> Self {
        Value::Num(n as f64)
    }
}
impl From<u64> for Value {
    fn from(n: u64) -> Self {
        Value::Num(n as f64)
    }
}
impl From<i32> for Value {
    fn from(n: i32) -> Self {
        Value::Num(n as f64)
    }
}
impl From<u32> for Value {
    fn from(n: u32) -> Self {
        Value::Num(n as f64)
    }
}
impl From<usize> for Value {
    fn from(n: usize) -> Self {
        Value::Num(n as f64)
    }
}
impl From<Map> for Value {
    fn from(m: Map) -> Self {
        Value::Obj(m)
    }
}
impl From<Vec<Value>> for Value {
    fn from(a: Vec<Value>) -> Self {
        Value::Arr(a)
    }
}
impl<T: Into<Value>> From<Option<T>> for Value {
    fn from(o: Option<T>) -> Self {
        o.map(Into::into).unwrap_or(Value::Null)
    }
}

/// `obj!{ "a" => 1, "b" => "x" }` monta um objeto na ordem escrita.
#[macro_export]
macro_rules! obj {
    () => { $crate::json::Value::Obj($crate::json::Map::new()) };
    ($($k:expr => $v:expr),+ $(,)?) => {{
        let mut m = $crate::json::Map::new();
        $( m.insert($k, $crate::json::Value::from($v)); )+
        $crate::json::Value::Obj(m)
    }};
}

// ---------------------------------------------------------------------------
// Number -> string (Number.prototype.toString, ECMA-262 Number::toString)
// ---------------------------------------------------------------------------
pub fn num_to_string(x: f64) -> String {
    if x.is_nan() {
        return "NaN".into();
    }
    if x == 0.0 {
        return "0".into(); // inclui -0
    }
    if x.is_infinite() {
        return if x > 0.0 { "Infinity".into() } else { "-Infinity".into() };
    }
    let neg = x < 0.0;
    // {:e} do Rust dá os dígitos mais curtos que fazem round-trip (igual ao V8)
    let e = format!("{:e}", x.abs());
    let (mant, exp) = e.split_once('e').unwrap();
    let exp: i32 = exp.parse().unwrap();
    let digits: String = mant.chars().filter(|c| *c != '.').collect();
    let k = digits.len() as i32;
    let n = exp + 1;
    let mut s = String::new();
    if neg {
        s.push('-');
    }
    if k <= n && n <= 21 {
        s.push_str(&digits);
        for _ in 0..(n - k) {
            s.push('0');
        }
    } else if 0 < n && n <= 21 {
        s.push_str(&digits[..n as usize]);
        s.push('.');
        s.push_str(&digits[n as usize..]);
    } else if -6 < n && n <= 0 {
        s.push_str("0.");
        for _ in 0..(-n) {
            s.push('0');
        }
        s.push_str(&digits);
    } else {
        let e1 = n - 1;
        s.push_str(&digits[..1]);
        if k > 1 {
            s.push('.');
            s.push_str(&digits[1..]);
        }
        s.push('e');
        s.push(if e1 >= 0 { '+' } else { '-' });
        s.push_str(&e1.abs().to_string());
    }
    s
}

// ---------------------------------------------------------------------------
// stringify
// ---------------------------------------------------------------------------
fn quote(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

fn write_value(out: &mut String, v: &Value, indent: Option<&str>, level: usize) {
    match v {
        Value::Null | Value::Undef => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Num(n) => {
            if n.is_finite() {
                out.push_str(&num_to_string(*n))
            } else {
                out.push_str("null")
            }
        }
        Value::Str(s) => quote(out, s),
        Value::Arr(a) => {
            if a.is_empty() {
                out.push_str("[]");
                return;
            }
            out.push('[');
            for (i, item) in a.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                newline(out, indent, level + 1);
                write_value(out, item, indent, level + 1);
            }
            newline(out, indent, level);
            out.push(']');
        }
        Value::Obj(m) => {
            let entries: Vec<(&String, &Value)> = m.iter().filter(|(_, v)| !matches!(v, Value::Undef)).collect();
            if entries.is_empty() {
                out.push_str("{}");
                return;
            }
            out.push('{');
            for (i, (k, item)) in entries.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                newline(out, indent, level + 1);
                quote(out, k);
                out.push(':');
                if indent.is_some() {
                    out.push(' ');
                }
                write_value(out, item, indent, level + 1);
            }
            newline(out, indent, level);
            out.push('}');
        }
    }
}

fn newline(out: &mut String, indent: Option<&str>, level: usize) {
    if let Some(ind) = indent {
        out.push('\n');
        for _ in 0..level {
            out.push_str(ind);
        }
    }
}

/// `JSON.stringify(v)`
pub fn stringify(v: &Value) -> String {
    let mut s = String::new();
    write_value(&mut s, v, None, 0);
    s
}

/// `JSON.stringify(v, null, 2)`
pub fn stringify_pretty(v: &Value) -> String {
    let mut s = String::new();
    write_value(&mut s, v, Some("  "), 0);
    s
}

// ---------------------------------------------------------------------------
// parse (JSON.parse)
// ---------------------------------------------------------------------------
#[derive(Debug)]
pub struct ParseError(pub String);

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

struct P<'a> {
    s: &'a [u8],
    i: usize,
}

impl P<'_> {
    fn ws(&mut self) {
        while self.i < self.s.len() && matches!(self.s[self.i], b' ' | b'\t' | b'\n' | b'\r') {
            self.i += 1;
        }
    }
    fn err<T>(&self, msg: &str) -> Result<T, ParseError> {
        Err(ParseError(format!("{} na posição {}", msg, self.i)))
    }
    fn value(&mut self, depth: usize) -> Result<Value, ParseError> {
        if depth > 512 {
            return self.err("aninhamento profundo demais");
        }
        self.ws();
        match self.s.get(self.i) {
            None => self.err("fim inesperado"),
            Some(b'{') => {
                self.i += 1;
                let mut m = Map::new();
                self.ws();
                if self.s.get(self.i) == Some(&b'}') {
                    self.i += 1;
                    return Ok(Value::Obj(m));
                }
                loop {
                    self.ws();
                    if self.s.get(self.i) != Some(&b'"') {
                        return self.err("esperava string");
                    }
                    let k = self.string()?;
                    self.ws();
                    if self.s.get(self.i) != Some(&b':') {
                        return self.err("esperava ':'");
                    }
                    self.i += 1;
                    let v = self.value(depth + 1)?;
                    m.insert(k, v);
                    self.ws();
                    match self.s.get(self.i) {
                        Some(b',') => self.i += 1,
                        Some(b'}') => {
                            self.i += 1;
                            return Ok(Value::Obj(m));
                        }
                        _ => return self.err("esperava ',' ou '}'"),
                    }
                }
            }
            Some(b'[') => {
                self.i += 1;
                let mut a = Vec::new();
                self.ws();
                if self.s.get(self.i) == Some(&b']') {
                    self.i += 1;
                    return Ok(Value::Arr(a));
                }
                loop {
                    a.push(self.value(depth + 1)?);
                    self.ws();
                    match self.s.get(self.i) {
                        Some(b',') => self.i += 1,
                        Some(b']') => {
                            self.i += 1;
                            return Ok(Value::Arr(a));
                        }
                        _ => return self.err("esperava ',' ou ']'"),
                    }
                }
            }
            Some(b'"') => Ok(Value::Str(self.string()?)),
            Some(b't') => self.lit("true", Value::Bool(true)),
            Some(b'f') => self.lit("false", Value::Bool(false)),
            Some(b'n') => self.lit("null", Value::Null),
            Some(b'-' | b'0'..=b'9') => self.number(),
            Some(_) => self.err("token inesperado"),
        }
    }
    fn lit(&mut self, w: &str, v: Value) -> Result<Value, ParseError> {
        if self.s[self.i..].starts_with(w.as_bytes()) {
            self.i += w.len();
            Ok(v)
        } else {
            self.err("token inesperado")
        }
    }
    fn number(&mut self) -> Result<Value, ParseError> {
        let start = self.i;
        if self.s.get(self.i) == Some(&b'-') {
            self.i += 1;
        }
        match self.s.get(self.i) {
            Some(b'0') => self.i += 1,
            Some(b'1'..=b'9') => {
                while matches!(self.s.get(self.i), Some(b'0'..=b'9')) {
                    self.i += 1;
                }
            }
            _ => return self.err("número inválido"),
        }
        if self.s.get(self.i) == Some(&b'.') {
            self.i += 1;
            if !matches!(self.s.get(self.i), Some(b'0'..=b'9')) {
                return self.err("número inválido");
            }
            while matches!(self.s.get(self.i), Some(b'0'..=b'9')) {
                self.i += 1;
            }
        }
        if matches!(self.s.get(self.i), Some(b'e' | b'E')) {
            self.i += 1;
            if matches!(self.s.get(self.i), Some(b'+' | b'-')) {
                self.i += 1;
            }
            if !matches!(self.s.get(self.i), Some(b'0'..=b'9')) {
                return self.err("número inválido");
            }
            while matches!(self.s.get(self.i), Some(b'0'..=b'9')) {
                self.i += 1;
            }
        }
        let txt = std::str::from_utf8(&self.s[start..self.i]).unwrap();
        Ok(Value::Num(txt.parse::<f64>().unwrap_or(f64::NAN)))
    }
    fn hex4(&mut self) -> Result<u32, ParseError> {
        let h = self.s.get(self.i..self.i + 4).ok_or(ParseError("escape \\u curto".into()))?;
        let h = std::str::from_utf8(h).map_err(|_| ParseError("escape \\u inválido".into()))?;
        let n = u32::from_str_radix(h, 16).map_err(|_| ParseError("escape \\u inválido".into()))?;
        self.i += 4;
        Ok(n)
    }
    fn string(&mut self) -> Result<String, ParseError> {
        self.i += 1; // aspas de abertura
        let mut out: Vec<u8> = Vec::new();
        loop {
            let c = match self.s.get(self.i) {
                None => return self.err("string não terminada"),
                Some(c) => *c,
            };
            self.i += 1;
            match c {
                b'"' => break,
                b'\\' => {
                    let e = match self.s.get(self.i) {
                        None => return self.err("string não terminada"),
                        Some(e) => *e,
                    };
                    self.i += 1;
                    let ch: char = match e {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'/' => '/',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'u' => {
                            let hi = self.hex4()?;
                            if (0xD800..0xDC00).contains(&hi)
                                && self.s.get(self.i) == Some(&b'\\')
                                && self.s.get(self.i + 1) == Some(&b'u')
                            {
                                let save = self.i;
                                self.i += 2;
                                let lo = self.hex4()?;
                                if (0xDC00..0xE000).contains(&lo) {
                                    char::from_u32(0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00))
                                        .unwrap_or('\u{FFFD}')
                                } else {
                                    self.i = save;
                                    '\u{FFFD}'
                                }
                            } else {
                                // surrogate solto não existe em String Rust
                                char::from_u32(hi).unwrap_or('\u{FFFD}')
                            }
                        }
                        _ => return self.err("escape inválido"),
                    };
                    let mut buf = [0u8; 4];
                    out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                }
                c if c < 0x20 => return self.err("caractere de controle em string"),
                c => out.push(c),
            }
        }
        Ok(String::from_utf8_lossy(&out).into_owned())
    }
}

/// `JSON.parse(s)`
pub fn parse(s: &str) -> Result<Value, ParseError> {
    let mut p = P { s: s.as_bytes(), i: 0 };
    let v = p.value(0)?;
    p.ws();
    if p.i != p.s.len() {
        return p.err("lixo após o JSON");
    }
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_like_js() {
        assert_eq!(num_to_string(12.0), "12");
        assert_eq!(num_to_string(12.5), "12.5");
        assert_eq!(num_to_string(-0.0), "0");
        assert_eq!(num_to_string(0.1 + 0.2), "0.30000000000000004");
        assert_eq!(num_to_string(1e21), "1e+21");
        assert_eq!(num_to_string(1e20), "100000000000000000000");
        assert_eq!(num_to_string(1.5e-7), "1.5e-7");
        assert_eq!(num_to_string(0.000001), "0.000001");
        assert_eq!(num_to_string(123456789012.25), "123456789012.25");
        assert_eq!(num_to_string(1789761723000.0), "1789761723000");
    }

    #[test]
    fn roundtrip_and_order() {
        let v = parse(r#"{"b":1,"2":0,"a":[true,null,"x\né"],"1":{},"b":3}"#).unwrap();
        assert_eq!(stringify(&v), r#"{"1":{},"2":0,"b":3,"a":[true,null,"x\né"]}"#);
    }

    #[test]
    fn pretty_like_node() {
        let v =
            obj! {"port" => 8080, "rcon" => obj!{"host" => "127.0.0.1"}, "users" => Vec::<Value>::new(), "e" => obj!{}};
        assert_eq!(
            stringify_pretty(&v),
            "{\n  \"port\": 8080,\n  \"rcon\": {\n    \"host\": \"127.0.0.1\"\n  },\n  \"users\": [],\n  \"e\": {}\n}"
        );
    }

    #[test]
    fn undefined_like_js() {
        let v = obj! {"a" => Value::Undef, "b" => 1, "c" => Value::Arr(vec![Value::Undef])};
        assert_eq!(stringify(&v), r#"{"b":1,"c":[null]}"#);
        assert_eq!(stringify(&obj! {"a" => Value::Undef}), "{}");
        assert_eq!(stringify_pretty(&obj! {"a" => Value::Undef}), "{}");
    }

    #[test]
    fn escapes() {
        assert_eq!(stringify(&Value::from("a\"\\\u{1}\u{7f}/")), "\"a\\\"\\\\\\u0001\u{7f}/\"");
        assert!(parse("{\"a\":1,}").is_err());
        assert!(parse("[01]").is_err());
        assert_eq!(parse(" \"\\ud83d\\ude00\" ").unwrap(), Value::from("😀"));
    }
}
