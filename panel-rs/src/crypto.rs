//! Criptografia do login, idêntica à do server.js:
//! - senha: `crypto.scryptSync(senha, salt, 64)` (N=16384, r=8, p=1), com o salt
//!   hex usado como TEXTO (bytes UTF-8 da string), igual ao Node;
//! - sessão: HMAC-SHA256 em base64url sem padding.
//!
//! HMAC, PBKDF2 e o RNG vêm do `ring` (o mesmo que já entra pelo rustls). O
//! scrypt é a composição da RFC 7914 (PBKDF2-HMAC-SHA256 + ROMix/Salsa20/8),
//! validada contra os vetores da RFC e contra um hash gerado pelo Node.

use ring::{hmac, pbkdf2, rand::SecureRandom};
use std::num::NonZeroU32;

pub fn random_bytes(n: usize) -> Vec<u8> {
    let mut b = vec![0u8; n];
    ring::rand::SystemRandom::new().fill(&mut b).expect("RNG do sistema indisponível");
    b
}

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{:02x}", x)).collect()
}

pub fn random_hex(n: usize) -> String {
    hex(&random_bytes(n))
}

/// Comparação em tempo constante (equivalente ao crypto.timingSafeEqual após o
/// teste de tamanho que o server.js faz antes).
pub fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut d = 0u8;
    for (x, y) in a.iter().zip(b) {
        d |= x ^ y;
    }
    d == 0
}

pub fn base64url(b: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(b.len().div_ceil(3) * 4);
    for ch in b.chunks(3) {
        let n = (ch[0] as u32) << 16 | (*ch.get(1).unwrap_or(&0) as u32) << 8 | *ch.get(2).unwrap_or(&0) as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        if ch.len() > 1 {
            out.push(T[(n >> 6) as usize & 63] as char);
        }
        if ch.len() > 2 {
            out.push(T[n as usize & 63] as char);
        }
    }
    out
}

/// `crypto.createHmac('sha256', key).update(msg).digest('base64url')`
pub fn hmac_b64url(key: &str, msg: &str) -> String {
    let k = hmac::Key::new(hmac::HMAC_SHA256, key.as_bytes());
    base64url(hmac::sign(&k, msg.as_bytes()).as_ref())
}

fn salsa20_8(b: &mut [u32; 16]) {
    let mut x = *b;
    for _ in 0..4 {
        macro_rules! qr {
            ($a:expr, $b:expr, $c:expr, $d:expr) => {
                x[$b] ^= x[$a].wrapping_add(x[$d]).rotate_left(7);
                x[$c] ^= x[$b].wrapping_add(x[$a]).rotate_left(9);
                x[$d] ^= x[$c].wrapping_add(x[$b]).rotate_left(13);
                x[$a] ^= x[$d].wrapping_add(x[$c]).rotate_left(18);
            };
        }
        // colunas
        qr!(0, 4, 8, 12);
        qr!(5, 9, 13, 1);
        qr!(10, 14, 2, 6);
        qr!(15, 3, 7, 11);
        // linhas
        qr!(0, 1, 2, 3);
        qr!(5, 6, 7, 4);
        qr!(10, 11, 8, 9);
        qr!(15, 12, 13, 14);
    }
    for i in 0..16 {
        b[i] = b[i].wrapping_add(x[i]);
    }
}

/// scryptBlockMix: `b` tem 2r blocos de 16 palavras; resultado em `y`.
fn block_mix(b: &[u32], y: &mut [u32], r: usize) {
    let mut x = [0u32; 16];
    x.copy_from_slice(&b[(2 * r - 1) * 16..2 * r * 16]);
    for i in 0..2 * r {
        for j in 0..16 {
            x[j] ^= b[i * 16 + j];
        }
        salsa20_8(&mut x);
        // pares vão pra primeira metade, ímpares pra segunda
        let dst = if i % 2 == 0 { (i / 2) * 16 } else { (r + i / 2) * 16 };
        y[dst..dst + 16].copy_from_slice(&x);
    }
}

fn ro_mix(block: &mut [u8], n: usize, r: usize) {
    let words = 32 * r;
    let mut x: Vec<u32> = block.chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
    let mut v = vec![0u32; words * n];
    let mut y = vec![0u32; words];
    for i in 0..n {
        v[i * words..(i + 1) * words].copy_from_slice(&x);
        block_mix(&x, &mut y, r);
        std::mem::swap(&mut x, &mut y);
    }
    for _ in 0..n {
        let j = (x[(2 * r - 1) * 16] as usize) & (n - 1);
        for k in 0..words {
            x[k] ^= v[j * words + k];
        }
        block_mix(&x, &mut y, r);
        std::mem::swap(&mut x, &mut y);
    }
    for (i, w) in x.iter().enumerate() {
        block[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
    }
}

pub fn scrypt(password: &[u8], salt: &[u8], n: usize, r: usize, p: usize, dklen: usize) -> Vec<u8> {
    assert!(n.is_power_of_two() && n > 1 && r > 0 && p > 0);
    let one = NonZeroU32::new(1).unwrap();
    let mut b = vec![0u8; p * 128 * r];
    pbkdf2::derive(pbkdf2::PBKDF2_HMAC_SHA256, one, salt, password, &mut b);
    for chunk in b.chunks_exact_mut(128 * r) {
        ro_mix(chunk, n, r);
    }
    let mut out = vec![0u8; dklen];
    pbkdf2::derive(pbkdf2::PBKDF2_HMAC_SHA256, one, &b, password, &mut out);
    out
}

/// `crypto.scryptSync(password, salt, 64).toString('hex')` com os defaults do Node.
pub fn scrypt_node_hex(password: &str, salt: &str) -> String {
    hex(&scrypt(password.as_bytes(), salt.as_bytes(), 16384, 8, 1, 64))
}

/// `hashPassword` do server.js: salt = 16 bytes aleatórios em hex.
pub fn hash_password(password: &str) -> (String, String) {
    let salt = random_hex(16);
    let hash = scrypt_node_hex(password, &salt);
    (salt, hash)
}

/// `checkPassword` do server.js.
pub fn check_password(pw: &str, salt: &str, hash: &str) -> bool {
    if salt.is_empty() || hash.is_empty() {
        return false;
    }
    let c = scrypt_node_hex(pw, salt);
    c.len() == hash.len() && ct_eq(c.as_bytes(), hash.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrypt_rfc7914_vectors() {
        let v1 = scrypt(b"", b"", 16, 1, 1, 64);
        assert_eq!(
            hex(&v1),
            "77d6576238657b203b19ca42c18a0497f16b4844e3074ae8dfdffa3fede21442fcd0069ded0948f8326a753a0fc81f17e8d3e0fb2e0d3628cf35e20c38d18906"
        );
        let v2 = scrypt(b"password", b"NaCl", 1024, 8, 16, 64);
        assert_eq!(
            hex(&v2),
            "fdbabe1c9d3472007856e7190d01e9fe7c6ad7cbc8237830e77376634b3731622eaf30d92e22a3886ff109279d9830dac727afb94a83ee6d8360cbdfa2cc0640"
        );
    }

    #[test]
    fn scrypt_matches_node() {
        // node -e "console.log(require('crypto').scryptSync('craftbox', '00112233445566778899aabbccddeeff', 64).toString('hex'))"
        assert_eq!(
            scrypt_node_hex("craftbox", "00112233445566778899aabbccddeeff"),
            include_str!("../parity/fixtures/scrypt-node.txt").trim()
        );
        assert!(check_password(
            "craftbox",
            "00112233445566778899aabbccddeeff",
            include_str!("../parity/fixtures/scrypt-node.txt").trim()
        ));
        assert!(!check_password(
            "errada",
            "00112233445566778899aabbccddeeff",
            include_str!("../parity/fixtures/scrypt-node.txt").trim()
        ));
    }

    #[test]
    fn hmac_matches_node() {
        // node -e "console.log(require('crypto').createHmac('sha256','segredo').update('s:1:admin').digest('base64url'))"
        assert_eq!(hmac_b64url("segredo", "s:1:admin"), include_str!("../parity/fixtures/hmac-node.txt").trim());
    }

    #[test]
    fn b64url() {
        assert_eq!(base64url(b"f"), "Zg");
        assert_eq!(base64url(b"fo"), "Zm8");
        assert_eq!(base64url(b"foo"), "Zm9v");
        assert_eq!(base64url(&[0xfb, 0xff]), "-_8");
    }
}
