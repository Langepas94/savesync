//! Парсер `appcache/appinfo.vdf` (версии 39–41) — локальный кэш Steam
//! с метаданными приложений. Наш источник appid → имя (+ UFS-конфиги
//! облачных путей для генератора манифестов — TODO M1, generator).
//!
//! Формат (SteamDatabase/SteamAppInfo, MIT):
//! - header: u32 magic (low byte = версия, upper = 0x075644), u32 universe,
//!   v41: i64 offset string table;
//! - записи до appid == 0: u32 appid, u32 size, u32 infoState,
//!   u32 lastUpdated, u64 picsToken, 20B sha1, u32 changeNumber,
//!   v40+: 20B binarySha1, затем бинарное KV-дерево.

use std::io;
use std::path::Path;

use thiserror::Error;

use super::kv_binary::{KvError, Parser as KvParser, Value};

#[derive(Debug, Error)]
pub enum AppInfoError {
    #[error("bad magic {0:#010x}, expected 0x075644xx")]
    BadMagic(u32),
    #[error("unsupported appinfo version {0}")]
    UnsupportedVersion(u8),
    #[error(transparent)]
    Kv(#[from] KvError),
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error("unexpected end of file at {0}")]
    UnexpectedEof(usize),
}

/// Одна запись appinfo.
#[derive(Debug, Clone)]
pub struct AppInfoEntry {
    pub appid: u32,
    pub change_number: u32,
    pub last_updated: u32,
    /// Полное KV-дерево записи (корень обычно `appinfo`).
    pub tree: Vec<(String, Value)>,
}

impl AppInfoEntry {
    /// `appinfo/common/name` — отображаемое имя приложения.
    pub fn name(&self) -> Option<&str> {
        get_in(&self.tree, &["appinfo", "common", "name"])?.as_str()
    }
}

/// Case-insensitive доступ по цепочке ключей без обёртки в `Value::Map`.
pub fn get_in<'a>(mut map: &'a [(String, Value)], path: &[&str]) -> Option<&'a Value> {
    let mut cur: Option<&'a Value> = None;
    for (i, key) in path.iter().enumerate() {
        let v = &map.iter().find(|(k, _)| k.eq_ignore_ascii_case(key))?.1;
        if i + 1 == path.len() {
            cur = Some(v);
        } else {
            map = v.as_map()?;
        }
    }
    cur
}

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], AppInfoError> {
        if self.pos + n > self.data.len() {
            return Err(AppInfoError::UnexpectedEof(self.pos));
        }
        let s = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    fn u32(&mut self) -> Result<u32, AppInfoError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn u64(&mut self) -> Result<u64, AppInfoError> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
}

/// Парсит содержимое `appinfo.vdf`. Версии < 39 отклоняются
/// (устаревшие клиенты; см. reverseengineering.wiki — поддерживаем 39–41).
pub fn parse(data: &[u8]) -> Result<Vec<AppInfoEntry>, AppInfoError> {
    let mut r = Reader { data, pos: 0 };

    let magic = r.u32()?;
    // 0x075644xx — актуальные версии (38+), 0x065644xx — старые (36–37).
    let prefix = magic >> 8;
    if prefix != 0x075644 && prefix != 0x065644 {
        return Err(AppInfoError::BadMagic(magic));
    }
    let version = (magic & 0xFF) as u8;
    if !(39..=41).contains(&version) {
        return Err(AppInfoError::UnsupportedVersion(version));
    }
    let _universe = r.u32()?;

    // v41: ключи KV-деревьев — индексы в общей string table.
    let string_table = if version >= 41 {
        let offset = r.u64()? as usize;
        Some(read_string_table(data, offset)?)
    } else {
        None
    };

    let mut entries = Vec::new();
    loop {
        let appid = r.u32()?;
        if appid == 0 {
            break; // футер файла
        }
        let _size = r.u32()?;
        let _info_state = r.u32()?;
        let last_updated = r.u32()?;
        let _pics_token = r.u64()?;
        let _sha1 = r.take(20)?;
        let change_number = r.u32()?;
        if version >= 40 {
            let _binary_sha1 = r.take(20)?;
        }

        let mut kv = KvParser::new(&data[r.pos..], string_table.as_deref());
        let tree = kv.map()?;
        r.pos += kv.position();

        entries.push(AppInfoEntry {
            appid,
            change_number,
            last_updated,
            tree,
        });
    }
    Ok(entries)
}

fn read_string_table(data: &[u8], offset: usize) -> Result<Vec<String>, AppInfoError> {
    let mut r = Reader { data, pos: offset };
    let count = r.u32()? as usize;
    let mut strings = Vec::with_capacity(count);
    for _ in 0..count {
        let rest = &data[r.pos..];
        let end = rest
            .iter()
            .position(|&b| b == 0)
            .ok_or(AppInfoError::UnexpectedEof(r.pos))?;
        strings.push(String::from_utf8_lossy(&rest[..end]).into_owned());
        r.pos += end + 1;
    }
    Ok(strings)
}

/// Читает и парсит `appinfo.vdf` с диска.
pub fn load(path: &Path) -> Result<Vec<AppInfoEntry>, AppInfoError> {
    parse(&std::fs::read(path)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cstr(s: &str) -> Vec<u8> {
        [s.as_bytes(), &[0]].concat()
    }

    /// Минимальное KV-дерево записи: { "appinfo": { "common": { "name": .. } } }
    fn kv_tree_v40(name: &str) -> Vec<u8> {
        let mut b = Vec::new();
        b.push(0u8);
        b.extend(cstr("appinfo"));
        b.push(0);
        b.extend(cstr("common"));
        b.push(1);
        b.extend(cstr("name"));
        b.extend(cstr(name));
        b.push(8); // end common
        b.push(8); // end appinfo
        b.push(8); // end root
        b
    }

    fn app_record_v40(appid: u32, name: &str) -> Vec<u8> {
        let tree = kv_tree_v40(name);
        let size = (4 + 4 + 8 + 20 + 4 + 20 + tree.len()) as u32; // от infoState до конца
        let mut b = Vec::new();
        b.extend(appid.to_le_bytes());
        b.extend(size.to_le_bytes());
        b.extend(2u32.to_le_bytes()); // info_state
        b.extend(1_700_000_000u32.to_le_bytes()); // last_updated
        b.extend(0u64.to_le_bytes()); // pics token
        b.extend([0xAA; 20]); // sha1
        b.extend(12345u32.to_le_bytes()); // change_number
        b.extend([0xBB; 20]); // binary sha1 (v40+)
        b.extend(tree);
        b
    }

    fn appinfo_v40(records: &[Vec<u8>]) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend(0x07564428u32.to_le_bytes());
        b.extend(1u32.to_le_bytes()); // universe
        for r in records {
            b.extend(r);
        }
        b.extend(0u32.to_le_bytes()); // футер: appid 0
        b
    }

    #[test]
    fn parses_v40_appinfo() {
        let data = appinfo_v40(&[
            app_record_v40(1245620, "ELDEN RING"),
            app_record_v40(413150, "Stardew Valley"),
        ]);
        let entries = parse(&data).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].appid, 1245620);
        assert_eq!(entries[0].name(), Some("ELDEN RING"));
        assert_eq!(entries[1].appid, 413150);
        assert_eq!(entries[1].name(), Some("Stardew Valley"));
        assert_eq!(entries[0].change_number, 12345);
    }

    #[test]
    fn parses_v41_with_string_table() {
        // Таблица: 0="appinfo", 1="common", 2="name"
        let table: Vec<String> = ["appinfo", "common", "name"]
            .iter()
            .map(|s| s.to_string())
            .collect();

        // KV-дерево с ключами-индексами; значение-строка inline.
        let mut tree = Vec::new();
        tree.push(0u8);
        tree.extend(0u32.to_le_bytes()); // "appinfo"
        tree.push(0);
        tree.extend(1u32.to_le_bytes()); // "common"
        tree.push(1);
        tree.extend(2u32.to_le_bytes()); // "name"
        tree.extend(cstr("Hades"));
        tree.extend([8, 8, 8]);

        let mut record = Vec::new();
        let size = (4 + 4 + 8 + 20 + 4 + 20 + tree.len()) as u32;
        record.extend(244850u32.to_le_bytes());
        record.extend(size.to_le_bytes());
        record.extend(2u32.to_le_bytes());
        record.extend(1_700_000_000u32.to_le_bytes());
        record.extend(0u64.to_le_bytes());
        record.extend([0xAA; 20]);
        record.extend(7u32.to_le_bytes());
        record.extend([0xBB; 20]);
        record.extend(&tree);

        let header_len = 4 + 4 + 8;
        let table_offset = header_len + record.len() + 4;

        let mut b = Vec::new();
        b.extend(0x07564429u32.to_le_bytes());
        b.extend(1u32.to_le_bytes());
        b.extend((table_offset as u64).to_le_bytes());
        b.extend(&record);
        b.extend(0u32.to_le_bytes()); // футер
        b.extend((table.len() as u32).to_le_bytes());
        for s in &table {
            b.extend(cstr(s));
        }

        let entries = parse(&b).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].appid, 244850);
        assert_eq!(entries[0].name(), Some("Hades"));
    }

    #[test]
    fn rejects_bad_magic_and_old_versions() {
        assert!(matches!(
            parse(&0xDEADBEEFu32.to_le_bytes()),
            Err(AppInfoError::BadMagic(_))
        ));
        let mut v36 = Vec::new();
        v36.extend(0x06564424u32.to_le_bytes()); // версия 36 — не поддерживаем
        v36.extend(1u32.to_le_bytes());
        v36.extend(0u32.to_le_bytes());
        assert!(matches!(
            parse(&v36),
            Err(AppInfoError::UnsupportedVersion(36))
        ));
    }

    #[test]
    fn rejects_truncated() {
        let data = appinfo_v40(&[app_record_v40(1, "X")]);
        assert!(parse(&data[..data.len() - 5]).is_err());
    }
}
