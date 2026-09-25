//! Gerador de .zip mínimo (sem compressão: "stored"), pra empacotar os mods
//! de um servidor sem depender do comando `zip`, que não vem no sistema
//! instalado. Os .jar já são zip comprimidos, então comprimir de novo quase
//! não ganha nada e só gasta CPU.
//!
//! Escreve direto num arquivo: cabeçalho local com CRC/tamanhos zerados, dados,
//! e depois volta (seek) pra preencher. Nomes em UTF-8 (flag 11). Sem ZIP64:
//! recusa passar de 4 GiB ou 65535 entradas.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};

pub enum Source<'a> {
    Path(&'a str),
    Bytes(&'a [u8]),
}

fn crc_table() -> [u32; 256] {
    let mut t = [0u32; 256];
    for (i, slot) in t.iter_mut().enumerate() {
        let mut c = i as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
        }
        *slot = c;
    }
    t
}

fn crc_update(t: &[u32; 256], crc: u32, data: &[u8]) -> u32 {
    let mut c = !crc;
    for &b in data {
        c = t[((c ^ b as u32) & 0xff) as usize] ^ (c >> 8);
    }
    !c
}

/// Data/hora no formato do DOS (fixa: 2026-01-01 00:00 — o conteúdo é o que importa).
const DOS_TIME: u16 = 0;
const DOS_DATE: u16 = ((2026 - 1980) << 9) | (1 << 5) | 1;

fn too_big() -> io::Error {
    io::Error::other("o zip passaria de 4 GiB (sem suporte a ZIP64)")
}

/// Escreve `entries` (nome dentro do zip, origem) em `out`.
pub fn write_stored(out: &mut File, entries: &[(String, Source)]) -> io::Result<()> {
    if entries.len() > 0xFFFF {
        return Err(io::Error::other("arquivos demais pra um zip sem ZIP64"));
    }
    let t = crc_table();
    let mut central: Vec<u8> = Vec::new();
    let mut offset: u64 = 0;
    let mut buf = vec![0u8; 256 * 1024];
    for (name, src) in entries {
        let nb = name.as_bytes();
        let header_at = offset;
        // cabeçalho local (crc/tamanhos preenchidos depois)
        let mut h = Vec::with_capacity(30 + nb.len());
        h.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        h.extend_from_slice(&20u16.to_le_bytes()); // versão necessária
        h.extend_from_slice(&0x0800u16.to_le_bytes()); // nome em UTF-8
        h.extend_from_slice(&0u16.to_le_bytes()); // stored
        h.extend_from_slice(&DOS_TIME.to_le_bytes());
        h.extend_from_slice(&DOS_DATE.to_le_bytes());
        h.extend_from_slice(&[0u8; 12]); // crc, tamanho comprimido, tamanho original
        h.extend_from_slice(&(nb.len() as u16).to_le_bytes());
        h.extend_from_slice(&0u16.to_le_bytes()); // extra
        h.extend_from_slice(nb);
        out.write_all(&h)?;
        let (mut crc, mut size) = (0u32, 0u64);
        match src {
            Source::Bytes(b) => {
                crc = crc_update(&t, crc, b);
                size = b.len() as u64;
                out.write_all(b)?;
            }
            Source::Path(p) => {
                let mut f = File::open(p)?;
                loop {
                    let n = f.read(&mut buf)?;
                    if n == 0 {
                        break;
                    }
                    crc = crc_update(&t, crc, &buf[..n]);
                    size += n as u64;
                    out.write_all(&buf[..n])?;
                }
            }
        }
        if size > u32::MAX as u64 || header_at > u32::MAX as u64 {
            return Err(too_big());
        }
        let end = out.stream_position()?;
        out.seek(SeekFrom::Start(header_at + 14))?;
        out.write_all(&crc.to_le_bytes())?;
        out.write_all(&(size as u32).to_le_bytes())?;
        out.write_all(&(size as u32).to_le_bytes())?;
        out.seek(SeekFrom::Start(end))?;
        offset = end;

        central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        central.extend_from_slice(&((3u16 << 8) | 20).to_le_bytes()); // feito em Unix, v2.0
        central.extend_from_slice(&20u16.to_le_bytes());
        central.extend_from_slice(&0x0800u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&DOS_TIME.to_le_bytes());
        central.extend_from_slice(&DOS_DATE.to_le_bytes());
        central.extend_from_slice(&crc.to_le_bytes());
        central.extend_from_slice(&(size as u32).to_le_bytes());
        central.extend_from_slice(&(size as u32).to_le_bytes());
        central.extend_from_slice(&(nb.len() as u16).to_le_bytes());
        central.extend_from_slice(&[0u8; 8]); // extra, comentário, disco, atributos internos
        central.extend_from_slice(&(0o100644u32 << 16).to_le_bytes()); // -rw-r--r--
        central.extend_from_slice(&(header_at as u32).to_le_bytes());
        central.extend_from_slice(nb);
    }
    if offset + central.len() as u64 > u32::MAX as u64 {
        return Err(too_big());
    }
    out.write_all(&central)?;
    let n = entries.len() as u16;
    let mut e = Vec::with_capacity(22);
    e.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    e.extend_from_slice(&[0u8; 4]); // disco atual / disco do diretório
    e.extend_from_slice(&n.to_le_bytes());
    e.extend_from_slice(&n.to_le_bytes());
    e.extend_from_slice(&(central.len() as u32).to_le_bytes());
    e.extend_from_slice(&(offset as u32).to_le_bytes());
    e.extend_from_slice(&0u16.to_le_bytes());
    out.write_all(&e)?;
    out.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_known_value() {
        assert_eq!(crc_update(&crc_table(), 0, b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn unzip_reads_what_we_write() {
        let d = std::env::temp_dir().join(format!("cbrs-zip-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let src = d.join("a.jar");
        std::fs::write(&src, vec![7u8; 300_000]).unwrap();
        let z = d.join("t.zip");
        let mut f = File::create(&z).unwrap();
        let p = src.to_str().unwrap().to_string();
        write_stored(&mut f, &[("mods/a.jar".into(), Source::Path(&p)), ("LEIA-ME.txt".into(), Source::Bytes("olá".as_bytes()))]).unwrap();
        drop(f);
        // `unzip -t` valida CRC e estrutura (o unzip é dependência do sistema)
        if let Ok(o) = std::process::Command::new("unzip").arg("-t").arg(&z).output() {
            let out = String::from_utf8_lossy(&o.stdout);
            assert!(o.status.success(), "{}", out);
            assert!(out.contains("mods/a.jar") && out.contains("LEIA-ME.txt"), "{}", out);
        }
        let _ = std::fs::remove_dir_all(&d);
    }
}
