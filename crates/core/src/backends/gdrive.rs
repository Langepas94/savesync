//! Google Drive: flat immutable publications avoid concurrent directory/name creation races.
use crate::{backends::backend::Backend, sync::index_journal};
use std::{
    io::{self, Read},
    time::Duration,
};
const DEFAULT_BASE: &str = "https://www.googleapis.com";
const FOLDER_MIME: &str = "application/vnd.google-apps.folder";
const ROOT_SPACE: &str = "appDataFolder";
pub struct GoogleDrive {
    token: String,
    base_url: String,
    agent: ureq::Agent,
    cache: crate::sync::publication_cache::Cache,
}
impl GoogleDrive {
    pub fn new(token: impl Into<String>) -> Self {
        let mut backend = Self::with_base_url(token, DEFAULT_BASE);
        backend.cache = crate::sync::publication_cache::Cache::persistent();
        backend
    }
    pub fn with_base_url(token: impl Into<String>, base_url: impl Into<String>) -> Self {
        Self {
            token: token.into(),
            base_url: base_url.into(),
            agent: ureq::AgentBuilder::new()
                .timeout(Duration::from_secs(60))
                .redirects(0)
                .build(),
            cache: crate::sync::publication_cache::Cache::default(),
        }
    }
    fn auth(&self, request: ureq::Request) -> ureq::Request {
        request.set("Authorization", &format!("Bearer {}", self.token))
    }
    fn api_error(&self, operation: &str, err: ureq::Error) -> io::Error {
        if matches!(err, ureq::Error::Status(401, _)) {
            return io::Error::new(
                io::ErrorKind::PermissionDenied,
                crate::auth::credential_backend::TokenExpired,
            );
        }
        let kind = match err {
            ureq::Error::Status(403, _) => io::ErrorKind::PermissionDenied,
            ureq::Error::Status(404, _) => io::ErrorKind::NotFound,
            _ => io::ErrorKind::Other,
        };
        let detail = match err {
            ureq::Error::Status(code, _) => format!("HTTP {code}"),
            _ => "transport failure".into(),
        };
        io::Error::new(kind, format!("google drive {operation}: {detail}"))
    }
    fn list_children(&self, parent: &str) -> io::Result<Vec<(String, String, bool)>> {
        let mut out = Vec::new();
        let mut page: Option<String> = None;
        let mut tokens = std::collections::HashSet::new();
        loop {
            let mut request = self
                .auth(self.agent.get(&format!("{}/drive/v3/files", self.base_url)))
                .query("q", &format!("'{parent}' in parents and trashed = false"))
                .query("spaces", ROOT_SPACE)
                .query("fields", "nextPageToken,files(id,name,mimeType)")
                .query("pageSize", "1000");
            if let Some(token) = &page {
                request = request.query("pageToken", token);
            }
            let json: serde_json::Value = request
                .call()
                .map_err(|e| self.api_error("list", e))?
                .into_json()
                .map_err(io::Error::other)?;
            let files = json
                .get("files")
                .and_then(|files| files.as_array())
                .ok_or_else(|| io::Error::other("invalid Drive file listing"))?;
            for file in files {
                let id = file
                    .get("id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| io::Error::other("Drive file id missing"))?;
                let name = file
                    .get("name")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| io::Error::other("Drive file name missing"))?;
                out.push((
                    id.into(),
                    name.into(),
                    file.get("mimeType").and_then(|v| v.as_str()) == Some(FOLDER_MIME),
                ));
            }
            page = json
                .get("nextPageToken")
                .and_then(|token| token.as_str())
                .map(str::to_string);
            match &page {
                Some(token) if !token.is_empty() => {
                    if !tokens.insert(token.clone()) {
                        return Err(io::Error::other("repeated Drive pagination token"));
                    }
                }
                _ => break,
            }
        }
        Ok(out)
    }
    fn download_id(&self, id: &str) -> io::Result<Vec<u8>> {
        let response = self
            .auth(
                self.agent
                    .get(&format!("{}/drive/v3/files/{id}", self.base_url)),
            )
            .query("alt", "media")
            .call()
            .map_err(|e| self.api_error("download", e))?;
        let mut bytes = Vec::new();
        response
            .into_reader()
            .take(512 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > 512 * 1024 * 1024 {
            return Err(io::Error::other("Drive file too large"));
        }
        Ok(bytes)
    }
    fn legacy_ids(&self, relative: &str) -> io::Result<Vec<String>> {
        let mut parents: Vec<String> = vec![ROOT_SPACE.into()];
        for name in std::iter::once("SaveSync").chain(relative.split('/')) {
            let mut next = Vec::new();
            for parent in parents {
                for (id, child, _) in self.list_children(&parent)? {
                    if child == name {
                        next.push(id);
                    }
                }
            }
            parents = next;
        }
        Ok(parents)
    }
    fn create_media(&self, name: &str, data: &[u8]) -> io::Result<()> {
        let metadata = serde_json::json!({"name":name,"parents":[ROOT_SPACE],"mimeType":"application/octet-stream"});
        if data.len() <= 5 * 1024 * 1024 {
            let boundary = format!(
                "ss{}",
                blake3::hash(oauth2::CsrfToken::new_random().secret().as_bytes()).to_hex()
            );
            let mut body=format!("--{boundary}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n{metadata}\r\n--{boundary}\r\nContent-Type: application/octet-stream\r\n\r\n").into_bytes();
            body.extend_from_slice(data);
            body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
            self.auth(
                self.agent
                    .post(&format!("{}/upload/drive/v3/files", self.base_url)),
            )
            .query("uploadType", "multipart")
            .set(
                "Content-Type",
                &format!("multipart/related; boundary={boundary}"),
            )
            .send_bytes(&body)
            .map_err(|e| self.api_error("create publication", e))?;
        } else {
            let response = self
                .auth(
                    self.agent
                        .post(&format!("{}/upload/drive/v3/files", self.base_url)),
                )
                .query("uploadType", "resumable")
                .set("X-Upload-Content-Type", "application/octet-stream")
                .set("X-Upload-Content-Length", &data.len().to_string())
                .send_json(metadata)
                .map_err(|e| self.api_error("start upload", e))?;
            let location = response
                .header("Location")
                .ok_or_else(|| io::Error::other("upload location missing"))?;
            let url = url::Url::parse(location).map_err(io::Error::other)?;
            let base = url::Url::parse(&self.base_url).map_err(io::Error::other)?;
            if url.origin() != base.origin() {
                return Err(io::Error::other("unexpected upload origin"));
            }
            self.auth(self.agent.put(location))
                .set("Content-Type", "application/octet-stream")
                .send_bytes(data)
                .map_err(|e| self.api_error("complete upload", e))?;
        }
        Ok(())
    }
    fn state(&self) -> io::Result<index_journal::State> {
        let mut records = Vec::new();
        for (id, name, directory) in self.list_children(ROOT_SPACE)? {
            if !directory && name.starts_with(index_journal::PREFIX) {
                let bytes = self.cache.get(&name, || self.download_id(&id))?;
                records.push((name, bytes));
            }
        }
        let mut legacy = Vec::new();
        for id in self.legacy_ids("index.json")? {
            legacy.push(self.download_id(&id)?);
        }
        let state = index_journal::read(legacy, records)?;
        Ok(state)
    }
}
impl Backend for GoogleDrive {
    fn name(&self) -> &'static str {
        "google-drive"
    }
    fn put_snapshot(&self, game: &str, version: &str, data: &[u8]) -> io::Result<()> {
        let name = index_journal::snapshot_object(game, version, data)?;
        if self
            .list_children(ROOT_SPACE)?
            .iter()
            .any(|(_, existing, directory)| !directory && existing == &name)
        {
            return Ok(());
        }
        self.create_media(&name, data)
    }
    fn get_snapshot(&self, game: &str, version: &str) -> io::Result<Vec<u8>> {
        let prefix = index_journal::snapshot_prefix(game)?;
        index_journal::snapshot_name(game, version)?;
        let mut ids = self
            .list_children(ROOT_SPACE)?
            .into_iter()
            .filter(|(_, child, dir)| {
                !dir && index_journal::object_version(child, &prefix) == Some(version)
            })
            .map(|(id, name, _)| (id, Some(name)))
            .collect::<Vec<_>>();
        if ids.is_empty() {
            ids = self
                .legacy_ids(&format!("saves/{game}/{version}.zip"))?
                .into_iter()
                .map(|id| (id, None))
                .collect();
        }
        let mut result = None;
        for (id, name) in ids {
            let bytes = self.download_id(&id)?;
            if let Some(name) = name {
                index_journal::verify_object(&name, &bytes)?;
            }
            if result.as_ref().is_some_and(|previous| previous != &bytes) {
                return Err(io::Error::other("conflicting immutable snapshot objects"));
            }
            result = Some(bytes);
        }
        result.ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "snapshot not found"))
    }
    fn list_snapshots(&self, game: &str) -> io::Result<Vec<String>> {
        let prefix = index_journal::snapshot_prefix(game)?;
        let mut out = self
            .list_children(ROOT_SPACE)?
            .into_iter()
            .filter_map(|(_, name, dir)| {
                if dir {
                    None
                } else {
                    index_journal::object_version(&name, &prefix).map(str::to_string)
                }
            })
            .collect::<Vec<_>>();
        for folder in self.legacy_ids(&format!("saves/{game}"))? {
            for (_, name, dir) in self.list_children(&folder)? {
                if !dir {
                    if let Some(version) = name.strip_suffix(".zip") {
                        out.push(version.into());
                    }
                }
            }
        }
        out.sort();
        out.dedup();
        Ok(out)
    }
    fn get_index(&self) -> io::Result<Option<Vec<u8>>> {
        let state = self.state()?;
        if state.seen {
            Ok(Some(
                serde_json::to_vec(&state.index).map_err(io::Error::other)?,
            ))
        } else {
            Ok(None)
        }
    }
    fn put_index(&self, data: &[u8]) -> io::Result<()> {
        self.publish_index(data, "manual")
    }
    fn publish_index(&self, data: &[u8], writer: &str) -> io::Result<()> {
        if let Some((name, bytes)) = index_journal::prepare(&self.state()?, data, writer)? {
            self.create_media(&name, &bytes)?;
        }

        Ok(())
    }
}
#[cfg(test)]
#[path = "../tests/backends/gdrive.rs"]
mod tests;
