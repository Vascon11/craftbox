//! Pequenas funções que reproduzem o comportamento de built-ins do JS usados
//! pelo server.js (parseInt, toFixed, trim, String(x), encode/decodeURIComponent,
//! path.join). Cada uma existe porque a diferença apareceria na resposta.

use crate::json::{num_to_string, Value};

/// Espaços do `String.prototype.trim` (WhiteSpace + LineTerminator do ECMAScript).
pub fn is_js_ws(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{b}' | '\u{c}' | '\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}'
    )
}

pub fn trim(s: &str) -> &str {
    s.trim_matches(is_js_ws)
}

/// `parseInt(s, 10)`; `None` = NaN.
pub fn parse_int(s: &str) -> Option<f64> {
    let s = s.trim_start_matches(is_js_ws);
    let (neg, rest) = match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    let digits: &str = &rest[..rest.bytes().take_while(|b| b.is_ascii_digit()).count()];
    if digits.is_empty() {
        return None;
    }
    let n: f64 = digits.parse().ok()?;
    Some(if neg { -n } else { n })
}

/// `Number(s)` / `+s` para strings (decimal, Infinity, 0x/0o/0b); `None` = NaN.
pub fn to_number(s: &str) -> Option<f64> {
    let t = trim(s);
    if t.is_empty() {
        return Some(0.0);
    }
    let lower = t.to_ascii_lowercase();
    for (p, radix) in [("0x", 16), ("0o", 8), ("0b", 2)] {
        if let Some(r) = lower.strip_prefix(p) {
            return u64::from_str_radix(r, radix).ok().map(|n| n as f64);
        }
    }
    match t {
        "Infinity" | "+Infinity" => return Some(f64::INFINITY),
        "-Infinity" => return Some(f64::NEG_INFINITY),
        _ => {}
    }
    // a gramática decimal do JS é um subconjunto do que o Rust aceita, exceto
    // "inf"/"nan", que o JS rejeita
    if t.bytes().any(|b| b.is_ascii_alphabetic() && b != b'e' && b != b'E') {
        return None;
    }
    t.parse::<f64>().ok()
}

/// Valor numérico de `+x.toFixed(d)` (arredondamento "meio pra cima" sobre o
/// valor binário exato, como o ECMAScript; `format!` do Rust faria meio-par).
pub fn to_fixed(x: f64, d: usize) -> f64 {
    if !x.is_finite() || x.abs() >= 1e21 {
        return x;
    }
    let neg = x < 0.0;
    let exact = format!("{:.1100}", x.abs()); // expansão decimal exata do double
    let (int_part, frac) = exact.split_once('.').unwrap();
    let mut digits: Vec<u8> = int_part.bytes().chain(frac.bytes().take(d)).map(|b| b - b'0').collect();
    let next = frac.as_bytes().get(d).map(|b| b - b'0').unwrap_or(0);
    if next >= 5 {
        let mut i = digits.len();
        loop {
            if i == 0 {
                digits.insert(0, 1);
                break;
            }
            i -= 1;
            if digits[i] == 9 {
                digits[i] = 0;
            } else {
                digits[i] += 1;
                break;
            }
        }
    }
    let int_len = digits.len() - d;
    let mut s = String::new();
    for (i, dg) in digits.iter().enumerate() {
        if i == int_len {
            s.push('.');
        }
        s.push((b'0' + dg) as char);
    }
    let v: f64 = s.parse().unwrap_or(0.0);
    if neg {
        -v
    } else {
        v
    }
}

/// `Math.round`
pub fn math_round(x: f64) -> f64 {
    (x + 0.5).floor()
}

/// Truthiness do JS.
pub fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Num(n)) => *n != 0.0 && !n.is_nan(),
        Some(Value::Str(s)) => !s.is_empty(),
        Some(Value::Arr(_)) | Some(Value::Obj(_)) => true,
    }
}

/// `String(x)` (undefined vira "undefined").
pub fn to_string(v: Option<&Value>) -> String {
    match v {
        None => "undefined".into(),
        Some(Value::Null) => "null".into(),
        Some(Value::Bool(b)) => b.to_string(),
        Some(Value::Num(n)) => num_to_string(*n),
        Some(Value::Str(s)) => s.clone(),
        Some(Value::Arr(a)) => a
            .iter()
            .map(|x| match x {
                Value::Null => String::new(),
                other => to_string(Some(other)),
            })
            .collect::<Vec<_>>()
            .join(","),
        Some(Value::Obj(_)) => "[object Object]".into(),
    }
}

/// `String(x || '')`
pub fn to_string_or_empty(v: Option<&Value>) -> String {
    if truthy(v) {
        to_string(v)
    } else {
        String::new()
    }
}

const URI_UNRESERVED: &[u8] = b"-_.!~*'()";

/// `encodeURIComponent`
pub fn encode_uri_component(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || URI_UNRESERVED.contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{:02X}", b));
        }
    }
    out
}

/// `decodeURIComponent` (Err = URIError "URI malformed").
pub fn decode_uri_component(s: &str) -> Result<String, ()> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let h = b.get(i + 1..i + 3).ok_or(())?;
            let h = std::str::from_utf8(h).map_err(|_| ())?;
            let v = u8::from_str_radix(h, 16).map_err(|_| ())?;
            out.push(v);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    // sequência UTF-8 inválida também é URIError no JS
    String::from_utf8(out).map_err(|_| ())
}

/// `path.join` (POSIX): junta e normaliza, preservando a barra final.
pub fn path_join(parts: &[&str]) -> String {
    let joined = parts.iter().filter(|p| !p.is_empty()).cloned().collect::<Vec<_>>().join("/");
    if joined.is_empty() {
        return ".".into();
    }
    path_normalize(&joined)
}

pub fn path_normalize(p: &str) -> String {
    let absolute = p.starts_with('/');
    let trailing = p.ends_with('/');
    let mut out: Vec<&str> = Vec::new();
    for seg in p.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                if out.last().is_some_and(|l| *l != "..") {
                    out.pop();
                } else if !absolute {
                    out.push("..");
                }
            }
            s => out.push(s),
        }
    }
    let mut s = out.join("/");
    if absolute {
        s.insert(0, '/');
    }
    if s.is_empty() {
        s = ".".into();
    }
    if trailing && s != "/" {
        s.push('/');
    }
    s
}

pub fn path_dirname(p: &str) -> String {
    let t = p.trim_end_matches('/');
    if t.is_empty() {
        return if p.starts_with('/') { "/".into() } else { ".".into() };
    }
    match t.rfind('/') {
        None => ".".into(),
        Some(i) => {
            let d = t[..i].trim_end_matches('/');
            if d.is_empty() {
                "/".into()
            } else {
                d.to_string()
            }
        }
    }
}

pub fn path_basename(p: &str) -> String {
    let t = p.trim_end_matches('/');
    match t.rfind('/') {
        None => t.to_string(),
        Some(i) => t[i + 1..].to_string(),
    }
}

/// `path.extname`
pub fn path_extname(p: &str) -> String {
    let base = path_basename(p);
    match base.rfind('.') {
        None | Some(0) => String::new(),
        Some(i) => base[i..].to_string(),
    }
}

/// `os.homedir()`: $HOME, senão o passwd.
pub fn homedir() -> String {
    if let Ok(h) = std::env::var("HOME") {
        if !h.is_empty() {
            return h;
        }
    }
    unsafe {
        let pw = libc::getpwuid(libc::getuid());
        if !pw.is_null() && !(*pw).pw_dir.is_null() {
            return std::ffi::CStr::from_ptr((*pw).pw_dir).to_string_lossy().into_owned();
        }
    }
    "/".into()
}

/// `Date.now()`
pub fn now_ms() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as f64).unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_int_js() {
        assert_eq!(parse_int("  8081abc"), Some(8081.0));
        assert_eq!(parse_int("-3"), Some(-3.0));
        assert_eq!(parse_int("abc"), None);
        assert_eq!(parse_int(""), None);
        assert_eq!(parse_int("45000\n"), Some(45000.0));
    }

    #[test]
    fn to_fixed_js() {
        assert_eq!(to_fixed(0.25, 1), 0.3); // JS: "0.3" (Rust {:.1} daria 0.2)
        assert_eq!(to_fixed(1.005, 2), 1.0); // 1.005 é 1.00499999... em binário
        assert_eq!(to_fixed(9.96, 1), 10.0);
        assert_eq!(to_fixed(-2.35, 1), -2.4); // -2.35 = -2.35000000000000008882
        assert_eq!(to_fixed(0.05, 1), 0.1);
        assert_eq!(to_fixed(123.456, 0), 123.0);
    }

    #[test]
    fn number_js() {
        assert_eq!(to_number(" 123 "), Some(123.0));
        assert_eq!(to_number(""), Some(0.0));
        assert_eq!(to_number("12abc"), None);
        assert_eq!(to_number("inf"), None);
        assert_eq!(to_number("1e3"), Some(1000.0));
    }

    #[test]
    fn uri_component() {
        assert_eq!(encode_uri_component("a@b c/ç"), "a%40b%20c%2F%C3%A7");
        assert_eq!(decode_uri_component("a%40b%20c%2F%C3%A7").unwrap(), "a@b c/ç");
        assert!(decode_uri_component("%E0").is_err());
        assert!(decode_uri_component("%zz").is_err());
    }

    #[test]
    fn paths() {
        assert_eq!(path_join(&["/srv/pub", "/app.js"]), "/srv/pub/app.js");
        assert_eq!(path_join(&["/srv/pub", ""]), "/srv/pub");
        assert_eq!(path_join(&["/srv/pub", "a/"]), "/srv/pub/a/");
        assert_eq!(path_join(&["/srv/pub", "../x"]), "/srv/x");
        assert_eq!(path_dirname("/srv/minecraft/servers"), "/srv/minecraft");
        assert_eq!(path_dirname("/srv"), "/");
        assert_eq!(path_dirname("servers"), ".");
        assert_eq!(path_basename("minecraft@alpha"), "minecraft@alpha");
        assert_eq!(path_extname("/x/logo.PNG"), ".PNG");
        assert_eq!(path_extname("/x/.hidden"), "");
    }

    #[test]
    fn trim_js() {
        assert_eq!(trim("\u{feff} a \r\n"), "a");
        assert_eq!(trim("\u{85}a"), "\u{85}a"); // NEL não é espaço no JS
    }
}
