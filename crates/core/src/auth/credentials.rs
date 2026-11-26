//! OS credential storage. Config contains opaque references, never token values.
use crate::auth::oauth::{Provider, TokenRecord};
use serde::{Deserialize, Serialize};
use std::io;

pub trait SecretStore: Send + Sync {
    fn get(&self, key: &str) -> io::Result<String>;
    fn put(&self, key: &str, value: &str) -> io::Result<()>;
    fn delete(&self, key: &str) -> io::Result<()>;
}
pub struct OsSecretStore;
mod file_store;
fn store_error(error: keyring::Error) -> io::Error {
    if matches!(error, keyring::Error::NoEntry) {
        io::Error::new(io::ErrorKind::NotFound, "credential is not stored")
    } else {
        io::Error::new(io::ErrorKind::PermissionDenied,"system credential store unavailable or locked; unlock Credential Manager/Secret Service")
    }
}
impl SecretStore for OsSecretStore {
    fn get(&self, key: &str) -> io::Result<String> {
        if let Some(store) = file_store::FileStore::from_environment()? {
            return store.get(key);
        }
        keyring::Entry::new("SaveSync", key)
            .map_err(store_error)?
            .get_password()
            .map_err(store_error)
    }
    fn put(&self, key: &str, value: &str) -> io::Result<()> {
        if let Some(store) = file_store::FileStore::from_environment()? {
            return store.put(key, value);
        }
        keyring::Entry::new("SaveSync", key)
            .map_err(store_error)?
            .set_password(value)
            .map_err(store_error)
    }
    fn delete(&self, key: &str) -> io::Result<()> {
        if let Some(store) = file_store::FileStore::from_environment()? {
            return store.delete(key);
        }
        keyring::Entry::new("SaveSync", key)
            .map_err(store_error)?
            .delete_credential()
            .map_err(store_error)
    }
}

#[derive(Serialize, Deserialize)]
struct Metadata {
    provider: Provider,
    client_id: Option<String>,
    expires_at: Option<u64>,
    revision: String,
    has_refresh: bool,
    has_secret: bool,
    scope: Option<String>,
}

fn nonce() -> String {
    blake3::hash(oauth2::CsrfToken::new_random().secret().as_bytes())
        .to_hex()
        .to_string()
}
fn reference_id(reference: &str) -> io::Result<&str> {
    let id = reference.strip_prefix("keyring:").ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "expected credential reference")
    })?;
    if id.len() != 64 || !id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid credential reference",
        ));
    }
    Ok(id)
}
pub fn is_reference(value: &str) -> bool {
    value.starts_with("keyring:")
}

/// Write a new revision before switching the metadata pointer. Split fields
/// avoid Windows Credential Manager's per-secret size limit for token bundles.
pub fn save(
    store: &dyn SecretStore,
    reference: Option<&str>,
    record: &TokenRecord,
) -> io::Result<String> {
    let reference = reference
        .map(str::to_string)
        .unwrap_or_else(|| format!("keyring:{}", nonce()));
    let id = reference_id(&reference)?;
    let old = match store.get(&format!("{id}/meta")) {
        Ok(text) => Some(serde_json::from_str::<Metadata>(&text)?),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    let revision = nonce();
    let prefix = format!("{id}/{revision}");
    let fields = [
        ("access", Some(record.access_token.as_str())),
        ("refresh", record.refresh_token.as_deref()),
        ("client-secret", record.client_secret.as_deref()),
    ];
    let result = (|| {
        for (field, value) in fields {
            if let Some(value) = value {
                store.put(&format!("{prefix}/{field}"), value)?;
            }
        }
        let metadata = Metadata {
            provider: record.provider,
            client_id: record.client_id.clone(),
            expires_at: record.expires_at,
            revision: revision.clone(),
            has_refresh: record.refresh_token.is_some(),
            has_secret: record.client_secret.is_some(),
            scope: record.scope.clone(),
        };
        store.put(&format!("{id}/meta"), &serde_json::to_string(&metadata)?)
    })();
    if let Err(error) = result {
        // A store can commit and then report a durability/transport error.
        // Never delete fields referenced by a pointer that may already be visible.
        let disposable = match store.get(&format!("{id}/meta")) {
            Ok(text) => {
                serde_json::from_str::<Metadata>(&text).is_ok_and(|meta| meta.revision != revision)
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => true,
            Err(_) => false,
        };
        if disposable {
            for (field, value) in fields {
                if value.is_some() {
                    let _ = store.delete(&format!("{prefix}/{field}"));
                }
            }
        }
        return Err(error);
    }
    if let Some(old) = old {
        for field in ["access", "refresh", "client-secret"] {
            let _ = store.delete(&format!("{id}/{}/{field}", old.revision));
        }
    }
    Ok(reference)
}

pub fn load(
    store: &dyn SecretStore,
    reference: &str,
    provider: Provider,
) -> io::Result<TokenRecord> {
    let id = reference_id(reference)?;
    let metadata: Metadata = serde_json::from_str(&store.get(&format!("{id}/meta"))?)?;
    if metadata.provider != provider {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "credential provider mismatch",
        ));
    }
    let prefix = format!("{id}/{}", metadata.revision);
    Ok(TokenRecord {
        provider,
        client_id: metadata.client_id,
        expires_at: metadata.expires_at,
        scope: metadata.scope,
        access_token: store.get(&format!("{prefix}/access"))?,
        refresh_token: if metadata.has_refresh {
            Some(store.get(&format!("{prefix}/refresh"))?)
        } else {
            None
        },
        client_secret: if metadata.has_secret {
            Some(store.get(&format!("{prefix}/client-secret"))?)
        } else {
            None
        },
    })
}

pub fn delete(store: &dyn SecretStore, reference: &str) -> io::Result<()> {
    let id = reference_id(reference)?;
    let metadata: Metadata = serde_json::from_str(&store.get(&format!("{id}/meta"))?)?;
    for field in ["access", "refresh", "client-secret"] {
        match store.delete(&format!("{id}/{}/{field}", metadata.revision)) {
            Ok(()) => (),
            Err(error) if error.kind() == io::ErrorKind::NotFound => (),
            Err(error) => return Err(error),
        }
    }
    store.delete(&format!("{id}/meta"))
}

pub fn secure_config(
    config: &crate::config::Config,
    store: &dyn SecretStore,
) -> io::Result<crate::config::Config> {
    let mut secured = config.clone();
    let fields = [
        (
            Provider::Google,
            secured
                .backends
                .google
                .as_mut()
                .map(|setting| &mut setting.token),
        ),
        (
            Provider::Yandex,
            secured
                .backends
                .yandex
                .as_mut()
                .map(|setting| &mut setting.token),
        ),
    ];
    let mut created = Vec::new();
    let outcome = (|| -> io::Result<()> {
        for (provider, field) in fields {
            if let Some(token) = field {
                if is_reference(token) {
                    reference_id(token)?;
                } else if !token.is_empty() {
                    *token = save(
                        store,
                        None,
                        &TokenRecord {
                            provider,
                            access_token: token.clone(),
                            refresh_token: None,
                            client_id: None,
                            client_secret: None,
                            expires_at: None,
                            scope: None,
                        },
                    )?;
                    created.push(token.clone());
                }
            }
        }
        Ok(())
    })();
    if let Err(error) = outcome {
        for reference in created {
            let _ = delete(store, &reference);
        }
        return Err(error);
    }
    Ok(secured)
}

#[cfg(test)]
#[path = "../tests/auth/credentials.rs"]
pub(crate) mod tests;

/// References are opaque identifiers, safe to compare without resolving secrets.
pub(crate) fn references(config: &crate::config::Config) -> Vec<&str> {
    config
        .backends
        .google
        .as_ref()
        .map(|value| value.token.as_str())
        .into_iter()
        .chain(
            config
                .backends
                .yandex
                .as_ref()
                .map(|value| value.token.as_str()),
        )
        .filter(|value| is_reference(value))
        .collect()
}
