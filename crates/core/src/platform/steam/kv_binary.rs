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
        // Реальный appinfo содержит не-UTF8 байты (имена в локальных
        // кодировках) — не падаем, заменяем битые последовательности.
        Ok(String::from_utf8_lossy(&rest[..end]).into_owned())
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
#[path = "../../tests/platform/kv_binary.rs"]
mod tests;
