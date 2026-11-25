//! Map a snapshot's logical save location onto this device.
use crate::{
    manifest::Manifest,
    paths::{resolve_pattern_multi, Platform, Roots},
    snapshot::{self, LocationManifest},
};
use std::path::PathBuf;

fn canonical(pattern: &str) -> String {
    let replacements = [
        ("AppData/Roaming/", "<winAppData>/"),
        ("AppData/Local/", "<winLocalAppData>/"),
        ("Saved Games/", "<winSavedGames>/"),
        ("Documents/", "<documents>/"),
    ];
    if pattern.starts_with("<steamCompat>/") {
        if let Some((_, tail)) = pattern.split_once("/pfx/drive_c/users/steamuser/") {
            for (prefix, placeholder) in replacements {
                if let Some(rest) = tail.strip_prefix(prefix) {
                    return format!("{placeholder}{rest}");
                }
            }
        }
    }
    pattern.to_string()
}

pub(super) fn resolve(
    manifest: &Manifest,
    roots: &Roots,
    platform: Platform,
    source: &LocationManifest,
) -> Option<(LocationManifest, PathBuf)> {
    let logical_id = source.location.as_ref().or_else(|| {
        manifest
            .paths
            .iter()
            .find(|rule| rule.pattern == source.pattern)
            .and_then(|rule| rule.location.as_ref())
    });
    let candidates: Vec<_> = manifest
        .paths_for(platform)
        .filter(|rule| {
            rule.pattern == source.pattern
                || (logical_id.is_some() && logical_id == rule.location.as_ref())
                || canonical(&rule.pattern) == canonical(&source.pattern)
        })
        .collect();
    // Prefer the exact rule; never guess between unrelated destinations.
    let rule = candidates
        .iter()
        .find(|r| r.pattern == source.pattern)
        .copied()
        .or_else(|| {
            if candidates.len() == 1 {
                Some(candidates[0])
            } else {
                None
            }
        })?;
    let resolved = resolve_pattern_multi(&rule.pattern, roots);
    let selected = resolved
        .iter()
        .find(|r| snapshot::location_base(r).exists())
        .or_else(|| resolved.first())?;
    let base = snapshot::location_base(selected);
    let mut mapped = source.clone();
    if rule.pattern != source.pattern {
        let source_has_glob = source.pattern.contains(['*', '?', '[']);
        let target_has_glob = rule.pattern.contains(['*', '?', '[']);
        let source_leaf = source.pattern.rsplit('/').next()?;
        let target_leaf = rule.pattern.rsplit('/').next()?;
        for file in &mut mapped.files {
            let relative = if source_has_glob {
                file.rel_path.as_str()
            } else {
                file.rel_path
                    .strip_prefix(source_leaf)?
                    .trim_start_matches('/')
            };
            file.rel_path = if target_has_glob {
                relative.to_string()
            } else if relative.is_empty() {
                target_leaf.to_string()
            } else {
                format!("{target_leaf}/{relative}")
            };
        }
    }
    Some((mapped, base))
}
