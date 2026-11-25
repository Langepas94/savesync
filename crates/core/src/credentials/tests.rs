use super::*;
use std::{collections::HashMap, sync::Mutex};
#[derive(Default)]
pub(crate) struct MemoryStore {
    pub values: Mutex<HashMap<String, String>>,
    pub fail_meta: Mutex<bool>,
}
impl SecretStore for MemoryStore {
    fn get(&self, key: &str) -> io::Result<String> {
        self.values
            .lock()
            .unwrap()
            .get(key)
            .cloned()
            .ok_or_else(|| io::ErrorKind::NotFound.into())
    }
    fn put(&self, key: &str, value: &str) -> io::Result<()> {
        if key.ends_with("/meta") && *self.fail_meta.lock().unwrap() {
            return Err(io::Error::other("injected store failure"));
        }
        self.values.lock().unwrap().insert(key.into(), value.into());
        Ok(())
    }
    fn delete(&self, key: &str) -> io::Result<()> {
        self.values
            .lock()
            .unwrap()
            .remove(key)
            .map(|_| ())
            .ok_or_else(|| io::ErrorKind::NotFound.into())
    }
}
pub(crate) fn record() -> TokenRecord {
    TokenRecord {
        provider: Provider::Google,
        access_token: "access-secret".into(),
        refresh_token: Some("refresh-secret".into()),
        client_id: Some("desktop-client".into()),
        client_secret: Some("client-secret".into()),
        expires_at: Some(123),
        scope: Some(Provider::Google.scope().into()),
    }
}
#[test]
fn revision_failure_keeps_previous_tokens() {
    let store = MemoryStore::default();
    let mut token = record();
    let reference = save(&store, None, &token).unwrap();
    let count = store.values.lock().unwrap().len();
    token.access_token = "replacement".into();
    *store.fail_meta.lock().unwrap() = true;
    assert!(save(&store, Some(&reference), &token).is_err());
    assert_eq!(
        load(&store, &reference, Provider::Google)
            .unwrap()
            .access_token,
        "access-secret"
    );
    assert_eq!(store.values.lock().unwrap().len(), count);
    *store.fail_meta.lock().unwrap() = false;
    assert_eq!(save(&store, Some(&reference), &token).unwrap(), reference);
    assert_eq!(
        load(&store, &reference, Provider::Google)
            .unwrap()
            .access_token,
        "replacement"
    );
    assert_eq!(store.values.lock().unwrap().len(), count);
    assert!(load(&store, &reference, Provider::Yandex).is_err());
    delete(&store, &reference).unwrap();
    assert!(store.values.lock().unwrap().is_empty());
}
#[test]
fn config_contains_only_reference_and_reuses_it() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("config.json");
    let mut config = crate::config::Config::load_or_init(&path).unwrap();
    config.backends.google = Some(crate::config::GoogleConfig {
        token: "private-access-token".into(),
    });
    let store = MemoryStore::default();
    config.save_with_store(&path, &store).unwrap();
    let bytes = std::fs::read_to_string(&path).unwrap();
    assert!(!bytes.contains("private-access-token"));
    let secured = crate::config::Config::load(&path).unwrap().unwrap();
    let reference = &secured.backends.google.as_ref().unwrap().token;
    assert!(is_reference(reference));
    assert_eq!(
        load(&store, reference, Provider::Google)
            .unwrap()
            .access_token,
        "private-access-token"
    );
    let count = store.values.lock().unwrap().len();
    secured.save_with_store(&path, &store).unwrap();
    assert_eq!(store.values.lock().unwrap().len(), count);
}
#[test]
fn config_write_fails_closed_when_store_locked() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("config.json");
    let mut config = crate::config::Config::load_or_init(&path).unwrap();
    let before = std::fs::read(&path).unwrap();
    config.backends.google = Some(crate::config::GoogleConfig {
        token: "must-not-leak".into(),
    });
    let store = MemoryStore::default();
    *store.fail_meta.lock().unwrap() = true;
    assert!(config.save_with_store(&path, &store).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert!(store.values.lock().unwrap().is_empty());
}

#[test]
fn removing_cloud_config_removes_its_credentials_after_commit() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("config.json");
    let mut config = crate::config::Config::load_or_init(&path).unwrap();
    config.backends.google = Some(crate::config::GoogleConfig {
        token: "access".into(),
    });
    let store = MemoryStore::default();
    config.save_with_store(&path, &store).unwrap();
    let mut config = crate::config::Config::load(&path).unwrap().unwrap();
    config.backends.google = None;
    config.save_with_store(&path, &store).unwrap();
    assert!(store.values.lock().unwrap().is_empty());
}
