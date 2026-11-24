//! Бинарное дерево KeyValues — payload записей `appinfo.vdf` /
//! `packageinfo.vdf`. Формат: [type u8][key][value], типы: 0=map,
//! 1=string, 2=int32, 7=uint64, 10=int64, 8=конец map.
//! Ключи: inline null-terminated UTF-8 (v40-) или u32-индекс в string
//! table (v41+). Строковые значения всегда inline.

use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum KvError {
    #[error("unexpected end of data at {0}")]
    UnexpectedEof(usize),
    #[error("unknown value type {0:#04x} at {1}")]
    UnknownType(u8, usize),
    #[error("string table index {0} out of range")]
    BadStringIndex(u32),
    #[error("invalid utf-8 string at {0}")]
    InvalidUtf8(usize),
}

/// Узел бинарного KV-дерева.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Map(Vec<(String, Value)>),
    Str(String),
    I32(i32),
    U64(u64),
    I64(i64),
}

impl Value {
    /// Доступ по цепочке ключей (case-insensitive — ключи appinfo
    /// встречаются в разном регистре).
    pub fn get(&self, path: &[&str]) -> Option<&Value> {
        let mut cur = self;
        for key in path {
            cur = &cur
                .as_map()?
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(key))?
                .1;
        }
        Some(cur)
    }

    pub fn as_map(&self) -> Option<&[(String, Value)]> {
        match self {
            Value::Map(m) => Some(m),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }
}

pub struct Parser<'a> {
    data: &'a [u8],
    pos: usize,
    /// String table для v41+ (ключи — индексы). `None` — ключи inline.
    strings: Option<&'a [String]>,
}

impl<'a> Parser<'a> {
    pub fn new(data: &'a [u8], strings: Option<&'a [String]>) -> Self {
        Parser {
            data,
            pos: 0,
            strings,
        }
    }

    pub fn position(&self) -> usize {
        self.pos
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], KvError> {
        if self.pos + n > self.data.len() {
            return Err(KvError::UnexpectedEof(self.pos));
        }
        let s = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    fn u8(&mut self) -> Result<u8, KvError> {
        Ok(self.take(1)?[0])
    }

    fn u32(&mut self) -> Result<u32, KvError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn u64(&mut self) -> Result<u64, KvError> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }

    fn cstr(&mut self) -> Result<String, KvError> {
        let start = self.pos;
        let rest = &self.data[start..];
        let end = rest
            .iter()
            .position(|&b| b == 0)
            .ok_or(KvError::UnexpectedEof(start))?;
        self.pos += end + 1;
        String::from_utf8(rest[..end].to_vec()).map_err(|_| KvError::InvalidUtf8(start))
    }

    fn key(&mut self) -> Result<String, KvError> {
        match self.strings {
            Some(table) => {
                let idx = self.u32()?;
                table
                    .get(idx as usize)
                    .cloned()
                    .ok_or(KvError::BadStringIndex(idx))
            }
            None => self.cstr(),
        }
    }

    /// Парсит один map-узел (последовательность пар ключ/значение до 0x08).
    pub fn map(&mut self) -> Result<Vec<(String, Value)>, KvError> {
        let mut out = Vec::new();
        loop {
            let at = self.pos;
            let ty = self.u8()?;
            if ty == 8 {
                break;
            }
            let key = self.key()?;
            let value = match ty {
                0 => Value::Map(self.map()?),
                1 => Value::Str(self.cstr()?),
                2 => Value::I32(self.u32()? as i32),
                7 => Value::U64(self.u64()?),
                10 => Value::I64(self.u64()? as i64),
                other => return Err(KvError::UnknownType(other, at)),
            };
            out.push((key, value));
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cstr(s: &str) -> Vec<u8> {
        [s.as_bytes(), &[0]].concat()
    }

    /// Собирает бинарное KV-дерево (v40-стиль, inline-ключи):
    /// { "appinfo": { "common": { "name": "Elden Ring" }, "appid": 1245620 } }
    fn sample_tree_v40() -> Vec<u8> {
        let mut b = Vec::new();
        b.push(0u8); // map "appinfo"
        b.extend(cstr("appinfo"));
        {
            b.push(0); // map "common"
            b.extend(cstr("common"));
            {
                b.push(1); // string "name"
                b.extend(cstr("name"));
                b.extend(cstr("Elden Ring"));
            }
            b.push(8); // end common
            b.push(2); // i32 "appid"
            b.extend(cstr("appid"));
            b.extend(1245620i32.to_le_bytes());
            b.push(7); // u64 "size"
            b.extend(cstr("size"));
            b.extend(9_999_999_999u64.to_le_bytes());
        }
        b.push(8); // end appinfo
        b.push(8); // end root
        b
    }

    #[test]
    fn parses_inline_keys() {
        let data = sample_tree_v40();
        let mut p = Parser::new(&data, None);
        let tree = Value::Map(p.map().unwrap());
        let name = tree.get(&["appinfo", "common", "name"]).unwrap();
        assert_eq!(name.as_str(), Some("Elden Ring"));
        assert!(matches!(
            tree.get(&["appinfo", "appid"]).unwrap(),
            Value::I32(1245620)
        ));
        assert!(matches!(
            tree.get(&["appinfo", "size"]).unwrap(),
            Value::U64(9_999_999_999)
        ));
    }

    #[test]
    fn case_insensitive_lookup() {
        let data = sample_tree_v40();
        let mut p = Parser::new(&data, None);
        let tree = Value::Map(p.map().unwrap());
        assert!(tree.get(&["APPINFO", "Common", "NAME"]).is_some());
    }

    #[test]
    fn parses_string_table_keys_v41() {
        // v41: ключи — u32-индексы в таблице строк.
        let table = vec!["root".to_string(), "answer".to_string()];
        let mut b = Vec::new();
        b.push(0u8); // map, ключ = индекс 0
        b.extend(0u32.to_le_bytes());
        b.push(2); // i32, ключ = индекс 1
        b.extend(1u32.to_le_bytes());
        b.extend(42i32.to_le_bytes());
        b.push(8);
        b.push(8);

        let mut p = Parser::new(&b, Some(&table));
        let tree = Value::Map(p.map().unwrap());
        assert!(matches!(
            tree.get(&["root", "answer"]).unwrap(),
            Value::I32(42)
        ));
    }

    #[test]
    fn rejects_truncated_data() {
        let data = &sample_tree_v40()[..5];
        let mut p = Parser::new(data, None);
        assert!(p.map().is_err());
    }

    #[test]
    fn rejects_unknown_type() {
        let mut b = Vec::new();
        b.push(0x42u8);
        b.extend(cstr("key"));
        let mut p = Parser::new(&b, None);
        assert!(matches!(p.map(), Err(KvError::UnknownType(0x42, 0))));
    }
}
