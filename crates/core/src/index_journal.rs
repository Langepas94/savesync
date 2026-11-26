//! Append-only cloud publications: concurrent writers never replace another writer's history.
use crate::index::Index;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, io};
pub(crate) const PREFIX: &str = "savesync-index-v2-";
#[derive(Serialize, Deserialize)]
struct Publication {
    format: u32,
    writer: String,
    clock: u64,
    index: Index,
}
pub(crate) struct State {
    pub index: Index,
    pub clock: u64,
    pub seen: bool,
    owned_locks: BTreeMap<String, BTreeMap<String, crate::index::GameLock>>,
}
fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
pub(crate) fn parse(bytes: &[u8]) -> io::Result<Index> {
    let index: Index = serde_json::from_slice(bytes).map_err(io::Error::other)?;
    if index.version != 1 {
        return Err(invalid("unsupported index schema"));
    }
    Ok(index)
}
pub(crate) fn merge(target: &mut Index, source: &Index) -> io::Result<()> {
    for (key, game) in &source.games {
        let mut versions = std::collections::HashMap::new();
        for version in &game.versions {
            if let Some(previous) = versions.insert(&version.id, version) {
                if !previous.same_record(version) {
                    return Err(invalid("conflicting immutable version records"));
                }
            }
        }
        if let Some(existing) = target.games.get(key) {
            for version in &game.versions {
                if existing
                    .get(&version.id)
                    .is_some_and(|old| !old.same_record(version))
                {
                    return Err(invalid("conflicting immutable version records"));
                }
            }
        }
    }
    target.merge(source);
    Ok(())
}
pub(crate) fn encode(bytes: &[u8], writer: &str, clock: u64) -> io::Result<(String, Vec<u8>)> {
    if writer.is_empty() || writer.len() > 256 {
        return Err(invalid("invalid index writer"));
    }
    let publication = Publication {
        format: 2,
        writer: writer.into(),
        clock,
        index: parse(bytes)?,
    };
    let bytes = serde_json::to_vec(&publication).map_err(io::Error::other)?;
    let hash = blake3::hash(&bytes).to_hex();
    Ok((format!("{PREFIX}{hash}.json"), bytes))
}
pub(crate) fn read(legacy: Vec<Vec<u8>>, records: Vec<(String, Vec<u8>)>) -> io::Result<State> {
    let mut index = Index::new();
    let mut publications = Vec::new();
    let seen = !legacy.is_empty() || !records.is_empty();
    for bytes in legacy {
        merge(&mut index, &parse(&bytes)?)?;
    }
    for (name, bytes) in records {
        let hash = blake3::hash(&bytes).to_hex();
        if name != format!("{PREFIX}{hash}.json") {
            return Err(invalid("index publication checksum mismatch"));
        }
        let publication: Publication = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
        if publication.format != 2
            || publication.index.version != 1
            || publication.writer.is_empty()
        {
            return Err(invalid("unsupported index publication"));
        }
        publications.push((publication, hash.to_string()));
    }
    publications
        .sort_by(|(a, ha), (b, hb)| (a.clock, &a.writer, ha).cmp(&(b.clock, &b.writer, hb)));
    let mut latest = BTreeMap::new();
    let mut clock = 0;
    for (publication, _) in &publications {
        clock = clock.max(publication.clock);
        merge(&mut index, &publication.index)?;
        for (key, game) in &publication.index.games {
            index.game_mut(key).policy = game.policy;
        }
        latest.insert(publication.writer.as_str(), &publication.index);
    }
    // Each writer owns its lock state. Absence in its latest publication is a release.
    index
        .locks
        .retain(|_, lock| !latest.contains_key(lock.device.as_str()));
    for (writer, publication) in latest {
        for (game, lock) in &publication.locks {
            if lock.device != writer {
                continue;
            }
            if index
                .locks
                .get(game)
                .is_none_or(|previous| previous.heartbeat < lock.heartbeat)
            {
                index.locks.insert(game.clone(), lock.clone());
            }
        }
    }
    let mut owned_locks: BTreeMap<String, BTreeMap<String, crate::index::GameLock>> =
        BTreeMap::new();
    for (publication, _) in &publications {
        owned_locks.insert(
            publication.writer.clone(),
            publication
                .index
                .locks
                .iter()
                .filter(|(_, lock)| lock.device == publication.writer)
                .map(|(game, lock)| (game.clone(), lock.clone()))
                .collect(),
        );
    }
    Ok(State {
        index,
        clock,
        seen,
        owned_locks,
    })
}
pub(crate) fn snapshot_prefix(game: &str) -> io::Result<String> {
    crate::storage_paths::game_component(game)?;
    Ok(format!(
        "savesync-snapshot-v2-{}-",
        blake3::hash(game.as_bytes()).to_hex()
    ))
}
pub(crate) fn snapshot_name(game: &str, version: &str) -> io::Result<String> {
    crate::storage_paths::validate_version(version)?;
    Ok(format!("{}{version}.zip", snapshot_prefix(game)?))
}
#[cfg(test)]
pub(crate) mod tests;

pub(crate) fn snapshot_object(game: &str, version: &str, data: &[u8]) -> io::Result<String> {
    let name = snapshot_name(game, version)?;
    Ok(format!(
        "{}--{}.zip",
        name.trim_end_matches(".zip"),
        blake3::hash(data).to_hex()
    ))
}
pub(crate) fn object_version<'a>(name: &'a str, prefix: &str) -> Option<&'a str> {
    let (version, hash) = name
        .strip_prefix(prefix)?
        .strip_suffix(".zip")?
        .rsplit_once("--")?;
    (hash.len() == 64
        && hash.bytes().all(|b| b.is_ascii_hexdigit())
        && crate::storage_paths::validate_version(version).is_ok())
    .then_some(version)
}
pub(crate) fn verify_object(name: &str, data: &[u8]) -> io::Result<()> {
    let expected = name
        .strip_suffix(".zip")
        .and_then(|name| name.rsplit_once("--"))
        .map(|(_, hash)| hash)
        .ok_or_else(|| invalid("invalid snapshot object"))?;
    if expected != blake3::hash(data).to_hex().as_str() {
        return Err(invalid("snapshot object checksum mismatch"));
    }
    Ok(())
}

/// Publish only new versions/policy changes; lock state is the writer's full current state.
pub(crate) fn prepare(
    state: &State,
    bytes: &[u8],
    writer: &str,
) -> io::Result<Option<(String, Vec<u8>)>> {
    let incoming = parse(bytes)?;
    let mut checked = state.index.clone();
    merge(&mut checked, &incoming)?;
    let mut delta = Index::new();
    for (game, entry) in &incoming.games {
        let existing = state.index.game(game);
        let versions = entry
            .versions
            .iter()
            .filter(|version| {
                existing
                    .and_then(|existing| existing.get(&version.id))
                    .is_none()
            })
            .cloned()
            .collect::<Vec<_>>();
        if !versions.is_empty() || existing.is_none_or(|existing| existing.policy != entry.policy) {
            delta.games.insert(
                game.clone(),
                crate::index::GameIndex {
                    policy: entry.policy,
                    versions,
                },
            );
        }
    }
    delta.locks = incoming
        .locks
        .into_iter()
        .filter(|(_, lock)| lock.device == writer)
        .collect();
    let previous = state.owned_locks.get(writer).cloned().unwrap_or_else(|| {
        state
            .index
            .locks
            .iter()
            .filter(|(_, lock)| lock.device == writer)
            .map(|(game, lock)| (game.clone(), lock.clone()))
            .collect()
    });
    if state.seen && delta.games.is_empty() && delta.locks == previous {
        return Ok(None);
    }
    let clock = state
        .clock
        .checked_add(1)
        .ok_or_else(|| invalid("publication clock exhausted"))?;
    encode(
        &serde_json::to_vec(&delta).map_err(io::Error::other)?,
        writer,
        clock,
    )
    .map(Some)
}
