//! Validated wire identifiers and portable filesystem names.
use std::io;

pub fn game_component(key: &str) -> io::Result<String> {
    let slug = key.strip_prefix("custom:").unwrap_or(key);
    if !crate::game_key::is_valid_slug(slug) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid game key",
        ));
    }
    if key.starts_with("custom:") {
        Ok(format!("custom__{slug}"))
    } else if matches!(
        slug.to_ascii_lowercase().as_str(),
        "con" | "prn" | "aux" | "nul"
    ) || (slug.len() == 4
        && (slug.starts_with("com") || slug.starts_with("lpt"))
        && matches!(slug.as_bytes()[3], b'1'..=b'9'))
    {
        Ok(format!("game__{slug}"))
    } else {
        Ok(slug.to_string())
    }
}

pub fn validate_version(id: &str) -> io::Result<()> {
    if id.is_empty()
        || id.len() > 256
        || !id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid version id",
        ));
    }
    Ok(())
}

/// ZIP paths use '/' on every platform; reject Windows escapes on Linux too.
pub fn validate_relative(path: &str) -> io::Result<()> {
    if path.is_empty()
        || path.contains(['\\', ':', '\0', '<', '>', '"', '|', '?', '*'])
        || path.bytes().any(|b| b < 32)
        || path.split('/').any(|part| {
            let name = part.split('.').next().unwrap_or("").to_ascii_lowercase();
            part.is_empty()
                || part == "."
                || part == ".."
                || part.ends_with(['.', ' '])
                || matches!(name.as_str(), "con" | "prn" | "aux" | "nul")
                || (name.len() == 4
                    && (name.starts_with("com") || name.starts_with("lpt"))
                    && matches!(name.as_bytes()[3], b'1'..=b'9'))
        })
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unsafe snapshot path",
        ));
    }
    Ok(())
}
