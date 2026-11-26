//! Бэкенд Яндекс.Диска (plan §2.1): REST API `cloud-api.yandex.net`.
//!
//! Аутентификация — OAuth-токен в конфиге (`backends.yandex.token`).
//! Корень данных: `disk:/SaveSync`.
//!
//! Flat content-addressed snapshots and append-only index publications preserve
//! concurrent writers. The legacy index remains a read-only migration source.

use std::io::{self, Read};
use std::time::Duration;

const DEFAULT_BASE: &str = "https://cloud-api.yandex.net/v1/disk";

/// Канал синхронизации через Яндекс.Диск.
pub struct YandexDisk {
    token: String,
    cache: crate::sync::publication_cache::Cache,
    /// Корень на диске, например `disk:/SaveSync`.
    root: String,
    /// База REST API (в тестах подменяется mock-сервером).
    base_url: String,
    agent: ureq::Agent,
}

impl YandexDisk {
    pub fn new(token: impl Into<String>) -> Self {
        let mut backend = Self::with_base_url(token, DEFAULT_BASE);
        backend.cache = crate::sync::publication_cache::Cache::persistent();
        backend
    }

    pub fn with_base_url(token: impl Into<String>, base_url: impl Into<String>) -> Self {
        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(60))
            .build();
        YandexDisk {
            token: token.into(),
            cache: crate::sync::publication_cache::Cache::default(),
            root: "disk:/SaveSync".into(),
            base_url: base_url.into(),
            agent,
        }
    }

    /// Корень данных на диске (для тестов/отладки).
    pub fn set_root(&mut self, root: impl Into<String>) {
        self.root = root.into();
    }

    fn auth(&self, req: ureq::Request) -> ureq::Request {
        req.set("Authorization", &format!("OAuth {}", self.token))
    }

    /// Полный путь ресурса на диске: `<root>/<rel>`.
    fn disk_path(&self, rel: &str) -> String {
        if rel.is_empty() {
            self.root.clone()
        } else {
            format!("{}/{rel}", self.root)
        }
    }

    fn api_error(&self, op: &str, err: ureq::Error) -> io::Error {
        if matches!(&err, ureq::Error::Status(401, _)) {
            return io::Error::new(
                io::ErrorKind::PermissionDenied,
                crate::auth::credential_backend::TokenExpired,
            );
        }
        let kind = match &err {
            ureq::Error::Status(401 | 403, _) => io::ErrorKind::PermissionDenied,
            ureq::Error::Status(404, _) => io::ErrorKind::NotFound,
            ureq::Error::Status(_, _) => io::ErrorKind::Other,
            ureq::Error::Transport(_) => io::ErrorKind::ConnectionAborted,
        };
        io::Error::new(
            kind,
            format!(
                "yandex disk {op}: {}",
                match err {
                    ureq::Error::Status(status, _) => format!("HTTP {status}"),
                    _ => "transport failure".into(),
                }
            ),
        )
    }

    /// GET JSON с query-параметрами.
    fn get_json(
        &self,
        op: &str,
        path: &str,
        query: &[(&str, &str)],
    ) -> io::Result<serde_json::Value> {
        let mut req = self
            .auth(self.agent.get(&format!("{}{path}", self.base_url)))
            .set("Accept", "application/json");
        for (k, v) in query {
            req = req.query(k, v);
        }
        match req.call() {
            Ok(resp) => resp
                .into_json()
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e)),
            Err(e) => Err(self.api_error(op, e)),
        }
    }

    /// Создать директорию (409 «уже есть» — не ошибка).
    fn mkdir(&self, rel: &str) -> io::Result<()> {
        let req = self
            .auth(self.agent.put(&format!("{}/resources", self.base_url)))
            .query("path", &self.disk_path(rel));
        match req.call() {
            Ok(_) => Ok(()),
            Err(ureq::Error::Status(409, _)) => Ok(()),
            Err(e) => Err(self.api_error("mkdir", e)),
        }
    }

    /// Загрузка байтов по пути (перезапись).
    fn upload(&self, rel: &str, data: &[u8]) -> io::Result<()> {
        self.upload_with_overwrite(rel, data, "true")
    }

    fn upload_with_overwrite(&self, rel: &str, data: &[u8], overwrite: &str) -> io::Result<()> {
        let link = self.get_json(
            "upload-href",
            "/resources/upload",
            &[("path", &self.disk_path(rel)), ("overwrite", overwrite)],
        )?;
        let href = link
            .get("href")
            .and_then(|h| h.as_str())
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "no href in upload link"))?;
        // href указывает на uploader.yandex.net; в тестах — на mock.
        self.agent
            .put(href)
            .set("Content-Type", "application/octet-stream")
            .send_bytes(data)
            .map_err(|e| self.api_error("upload", e))?;
        Ok(())
    }

    pub fn verify_access(&self) -> io::Result<()> {
        self.mkdir("")?;
        let nonce = oauth2::CsrfToken::new_random_len(24);
        let name = format!("savesync-access-check-{}.tmp", nonce.secret());
        let data = b"SaveSync access check";
        self.upload_with_overwrite(&name, data, "false")?;
        let read = self.download(&name).and_then(|bytes| {
            if bytes == data {
                Ok(())
            } else {
                Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Проверочный файл Яндекс Диска повреждён",
                ))
            }
        });
        let cleanup = self
            .auth(self.agent.delete(&format!("{}/resources", self.base_url)))
            .query("path", &self.disk_path(&name))
            .query("permanently", "true")
            .call()
            .map(|_| ())
            .map_err(|error| self.api_error("delete access check", error));
        read.and(cleanup)
    }

    /// Скачивание байтов по пути.
    fn download(&self, rel: &str) -> io::Result<Vec<u8>> {
        let link = self.get_json(
            "download-href",
            "/resources/download",
            &[("path", &self.disk_path(rel))],
        )?;
        let href = link.get("href").and_then(|h| h.as_str()).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "no href in download link")
        })?;
        let resp = self
            .agent
            .get(href)
            .call()
            .map_err(|e| self.api_error("download", e))?;
        let mut buf = Vec::new();
        resp.into_reader()
            .take(512 * 1024 * 1024)
            .read_to_end(&mut buf)?;
        Ok(buf)
    }

    /// Read every page; truncating at 1000 objects would silently hide history.
    fn list_files(&self, relative: &str) -> io::Result<Vec<String>> {
        let mut offset = 0;
        let mut out = Vec::new();
        let mut seen_pages = std::collections::HashSet::new();
        loop {
            let json = match self.get_json(
                "list",
                "/resources",
                &[
                    ("path", &self.disk_path(relative)),
                    ("limit", "1000"),
                    ("offset", &offset.to_string()),
                    ("fields", "_embedded.items.name,_embedded.items.type"),
                ],
            ) {
                Ok(json) => json,
                Err(error) if error.kind() == io::ErrorKind::NotFound => break,
                Err(error) => return Err(error),
            };
            let items = json
                .get("_embedded")
                .and_then(|value| value.get("items"))
                .and_then(|value| value.as_array())
                .ok_or_else(|| io::Error::other("invalid Yandex resource listing"))?;
            let fingerprint = serde_json::to_string(items).map_err(io::Error::other)?;
            if !items.is_empty() && !seen_pages.insert(fingerprint) {
                return Err(io::Error::other("repeated Yandex listing page"));
            }
            for item in items {
                if item.get("type").and_then(|value| value.as_str()) == Some("file") {
                    out.push(
                        item.get("name")
                            .and_then(|value| value.as_str())
                            .ok_or_else(|| io::Error::other("resource name missing"))?
                            .into(),
                    );
                }
            }
            if items.len() < 1000 {
                break;
            }
            offset += items.len();
        }
        Ok(out)
    }
    fn publication_state(&self) -> io::Result<crate::sync::index_journal::State> {
        let legacy = match self.download("index.json") {
            Ok(bytes) => vec![bytes],
            Err(error) if error.kind() == io::ErrorKind::NotFound => vec![],
            Err(error) => return Err(error),
        };
        let mut records = Vec::new();
        for name in self.list_files("")? {
            if name.starts_with(crate::sync::index_journal::PREFIX) {
                let bytes = self.cache.get(&name, || self.download(&name))?;
                records.push((name, bytes));
            }
        }
        let state = crate::sync::index_journal::read(legacy, records)?;
        Ok(state)
    }
}

impl crate::backends::backend::Backend for YandexDisk {
    fn put_snapshot(&self, game: &str, version: &str, data: &[u8]) -> io::Result<()> {
        let name = crate::sync::index_journal::snapshot_object(game, version, data)?;
        if self.list_files("")?.contains(&name) {
            return Ok(());
        }
        self.mkdir("")?;
        self.upload(&name, data)
    }
    fn get_snapshot(&self, game: &str, version: &str) -> io::Result<Vec<u8>> {
        let prefix = crate::sync::index_journal::snapshot_prefix(game)?;
        crate::sync::index_journal::snapshot_name(game, version)?;
        let mut result = None;
        for name in self.list_files("")? {
            if crate::sync::index_journal::object_version(&name, &prefix) == Some(version) {
                let bytes = self.download(&name)?;
                crate::sync::index_journal::verify_object(&name, &bytes)?;
                if result.as_ref().is_some_and(|previous| previous != &bytes) {
                    return Err(io::Error::other("conflicting immutable snapshot objects"));
                }
                result = Some(bytes);
            }
        }
        match result {
            Some(bytes) => Ok(bytes),
            None => self.download(&format!("saves/{game}/{version}.zip")),
        }
    }
    fn list_snapshots(&self, game: &str) -> io::Result<Vec<String>> {
        let prefix = crate::sync::index_journal::snapshot_prefix(game)?;
        let mut versions = self
            .list_files("")?
            .into_iter()
            .filter_map(|name| {
                crate::sync::index_journal::object_version(&name, &prefix).map(str::to_string)
            })
            .collect::<Vec<_>>();
        versions.extend(
            self.list_files(&format!("saves/{game}"))?
                .into_iter()
                .filter_map(|name| name.strip_suffix(".zip").map(str::to_string)),
        );
        versions.sort();
        versions.dedup();
        Ok(versions)
    }
    fn get_index(&self) -> io::Result<Option<Vec<u8>>> {
        let state = self.publication_state()?;
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
        if let Some((name, bytes)) =
            crate::sync::index_journal::prepare(&self.publication_state()?, data, writer)?
        {
            self.mkdir("")?;
            self.upload(&name, &bytes)?;
        }

        Ok(())
    }

    fn name(&self) -> &'static str {
        "yandex-disk"
    }

    fn verify_access(&self) -> io::Result<()> {
        YandexDisk::verify_access(self)
    }
}

#[cfg(test)]
#[path = "../tests/backends/yandex.rs"]
mod tests;
