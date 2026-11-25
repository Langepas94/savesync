//! Map a snapshot's logical save location onto this device.
use crate::{
    manifest::Manifest,
    paths::{resolve_pattern_multi, windows_save_pattern, Platform, Roots},
    snapshot::{self, LocationManifest},
};
use std::path::PathBuf;

pub(super) fn resolve(
    manifest: &Manifest,
    roots: &Roots,
    platform: Platform,
    source: &LocationManifest,
) -> Option<(LocationManifest, PathBuf)> {
    let default_source_id = format!("windows/{}", windows_save_pattern(&source.pattern));
    let logical_id = source
        .location
        .as_deref()
        .or_else(|| {
            manifest
                .paths
                .iter()
                .find(|rule| rule.pattern == source.pattern)
                .and_then(|rule| rule.location.as_deref())
        })
        .unwrap_or(&default_source_id);
    let candidates: Vec<_> = manifest
        .paths_for(platform)
        .filter(|rule| {
            rule.pattern == source.pattern
                || rule.location.as_deref() == Some(logical_id)
                || (rule.location.is_none()
                    && logical_id == format!("windows/{}", windows_save_pattern(&rule.pattern)))
                || windows_save_pattern(&rule.pattern) == windows_save_pattern(&source.pattern)
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
    let resolved: Vec<_> = resolve_pattern_multi(&rule.pattern, roots)
        .iter()
        .map(|pattern| crate::paths::canonical_pattern(pattern))
        .collect();
    let selected = resolved
        .iter()
        .find(|r| snapshot::location_base(r).exists())
        .or_else(|| resolved.first())?;
    let base = snapshot::location_base(selected);
    let mut mapped = source.clone();
    if rule.pattern != source.pattern {
        let source_has_glob = snapshot::first_wildcard(&source.pattern).is_some();
        let target_has_glob = snapshot::first_wildcard(&rule.pattern).is_some();
        let source_leaf = snapshot::unescape_pattern(source.pattern.rsplit('/').next()?);
        let target_leaf = snapshot::unescape_pattern(rule.pattern.rsplit('/').next()?);
        for file in &mut mapped.files {
            let relative = if source_has_glob {
                file.rel_path.as_str()
            } else {
                file.rel_path
                    .strip_prefix(&source_leaf)?
                    .trim_start_matches('/')
            };
            file.rel_path = if target_has_glob {
                relative.to_string()
            } else if relative.is_empty() {
                target_leaf.clone()
            } else {
                format!("{target_leaf}/{relative}")
            };
        }
    }
    Some((mapped, base))
}
