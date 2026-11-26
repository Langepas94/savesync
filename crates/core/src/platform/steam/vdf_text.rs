//! Текстовый формат KeyValues (Valve VDF/ACF): `appmanifest_*.acf`,
//! `libraryfolders.vdf`, `config.vdf`.
//!
//! Грамматика: `"key" "value"` или `"key" { ... }`, комментарии `//`,
//! строки в двойных кавычках, `\"` — экранированная кавычка, остальные
//! backslash-последовательности литеральны (пути Windows не экранируются).

use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum VdfError {
    #[error("unexpected end of input")]
    UnexpectedEof,
    #[error("expected quoted string at byte {0}")]
    ExpectedString(usize),
    #[error("expected value or block at byte {0}")]
    ExpectedValue(usize),
    #[error("unexpected closing brace at byte {0}")]
    UnexpectedBrace(usize),
}

/// Узел текстового VDF.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Node {
    Map(Vec<(String, Node)>),
    Str(String),
}

impl Node {
    pub fn get(&self, path: &[&str]) -> Option<&Node> {
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

    pub fn as_map(&self) -> Option<&[(String, Node)]> {
        match self {
            Node::Map(m) => Some(m),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Node::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn get_str(&self, path: &[&str]) -> Option<&str> {
        self.get(path)?.as_str()
    }
}

/// Парсит документ целиком (одна или несколько корневых пар).
pub fn parse(text: &str) -> Result<Vec<(String, Node)>, VdfError> {
    let mut p = TextParser {
        bytes: text.as_bytes(),
        pos: 0,
    };
    let mut out = Vec::new();
    loop {
        p.skip_ws();
        if p.pos >= p.bytes.len() {
            break;
        }
        let key = p.quoted_string()?;
        out.push((key, p.value()?));
    }
    Ok(out)
}

struct TextParser<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> TextParser<'a> {
    fn skip_ws(&mut self) {
        loop {
            while self.pos < self.bytes.len() && self.bytes[self.pos].is_ascii_whitespace() {
                self.pos += 1;
            }
            // Комментарии // до конца строки
            if self.bytes[self.pos..].starts_with(b"//") {
                while self.pos < self.bytes.len() && self.bytes[self.pos] != b'\n' {
                    self.pos += 1;
                }
            } else {
                break;
            }
        }
    }

    fn quoted_string(&mut self) -> Result<String, VdfError> {
        let start = self.pos;
        if self.pos >= self.bytes.len() {
            return Err(VdfError::UnexpectedEof);
        }
        if self.bytes[self.pos] != b'"' {
            return Err(VdfError::ExpectedString(start));
        }
        self.pos += 1;
        let mut s = String::new();
        while self.pos < self.bytes.len() {
            match self.bytes[self.pos] {
                b'"' => {
                    self.pos += 1;
                    return Ok(s);
                }
                b'\\' if self.bytes.get(self.pos + 1) == Some(&b'"') => {
                    s.push('"');
                    self.pos += 2;
                }
                // Valve пишет backslash удвоенным ("C:\\Games") — разэкранируем.
                // Одиночный "\" перед прочими символами остаётся литералом.
                b'\\' if self.bytes.get(self.pos + 1) == Some(&b'\\') => {
                    s.push('\\');
                    self.pos += 2;
                }
                b => {
                    // VDF — почти всегда UTF-8; на невалидных байтах не падаем.
                    let ch_len = utf8_len(b);
                    let end = (self.pos + ch_len).min(self.bytes.len());
                    s.push_str(&String::from_utf8_lossy(&self.bytes[self.pos..end]));
                    self.pos = end;
                }
            }
        }
        Err(VdfError::UnexpectedEof)
    }

    fn value(&mut self) -> Result<Node, VdfError> {
        self.skip_ws();
        let at = self.pos;
        if self.pos >= self.bytes.len() {
            return Err(VdfError::UnexpectedEof);
        }
        match self.bytes[self.pos] {
            b'{' => {
                self.pos += 1;
                let mut items = Vec::new();
                loop {
                    self.skip_ws();
                    if self.pos >= self.bytes.len() {
                        return Err(VdfError::UnexpectedEof);
                    }
                    if self.bytes[self.pos] == b'}' {
                        self.pos += 1;
                        break;
                    }
                    let key = self.quoted_string()?;
                    items.push((key, self.value()?));
                }
                Ok(Node::Map(items))
            }
            b'}' => Err(VdfError::UnexpectedBrace(at)),
            b'"' => Ok(Node::Str(self.quoted_string()?)),
            _ => Err(VdfError::ExpectedValue(at)),
        }
    }
}

fn utf8_len(first: u8) -> usize {
    match first {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        _ => 4,
    }
}

#[cfg(test)]
#[path = "../../tests/platform/vdf_text.rs"]
mod tests;
