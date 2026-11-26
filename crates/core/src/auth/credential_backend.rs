//! Retry an expired cloud access token once, rebuilding its Backend adapter.
use crate::{
    backend::Backend,
    oauth::{self, Provider},
};
use std::{cell::RefCell, io};

#[derive(Debug)]
pub struct TokenExpired;
impl std::fmt::Display for TokenExpired {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("cloud access token expired (HTTP 401)")
    }
}
impl std::error::Error for TokenExpired {}

type Factory = Box<dyn Fn(&str) -> Box<dyn Backend>>;
pub struct CredentialBackend {
    current: RefCell<Box<dyn Backend>>,
    factory: Factory,
    refresh: Box<dyn Fn() -> io::Result<String>>,
    name: &'static str,
}
impl CredentialBackend {
    /// Keep a failed cloud channel visible without disabling independent channels.
    pub fn connect(provider: Provider, reference: &str) -> Box<dyn Backend> {
        match Self::new(provider, reference) {
            Ok(backend) => Box::new(backend),
            Err(error) => Box::new(UnavailableBackend {
                name: match provider {
                    Provider::Google => "google-drive",
                    Provider::Yandex => "yandex-disk",
                },
                message: error.to_string(),
            }),
        }
    }
    pub fn new(provider: Provider, reference: &str) -> io::Result<Self> {
        let record = oauth::access_record(reference, provider, false)?;
        let token = record.access_token;
        let app_folder = provider == Provider::Yandex
            && record.scope.as_ref().is_some_and(|scope| {
                scope
                    .split_whitespace()
                    .any(|scope| scope == provider.scope())
            });
        let factory: Factory = Box::new(move |token| match provider {
            Provider::Google => Box::new(crate::backends::gdrive::GoogleDrive::new(token)),
            Provider::Yandex => {
                let mut backend = crate::backends::yandex::YandexDisk::new(token);
                if app_folder {
                    backend.set_root("app:/SaveSync");
                }
                Box::new(backend)
            }
        });
        let name = match provider {
            Provider::Google => "google-drive",
            Provider::Yandex => "yandex-disk",
        };
        let current = RefCell::new(factory(&token));
        let reference = reference.to_string();
        let refresh = Box::new(move || oauth::access_token(&reference, provider, true));
        Ok(Self {
            current,
            factory,
            refresh,
            name,
        })
    }
    fn execute<T>(&self, operation: impl Fn(&dyn Backend) -> io::Result<T>) -> io::Result<T> {
        let result = operation(self.current.borrow().as_ref());
        if result
            .as_ref()
            .err()
            .and_then(io::Error::get_ref)
            .is_some_and(|error| error.is::<TokenExpired>())
        {
            let token = (self.refresh)()?;
            *self.current.borrow_mut() = (self.factory)(&token);
            return operation(self.current.borrow().as_ref());
        }
        result
    }
}
impl Backend for CredentialBackend {
    fn name(&self) -> &'static str {
        self.name
    }
    fn put_snapshot(&self, g: &str, v: &str, data: &[u8]) -> io::Result<()> {
        self.execute(|backend| backend.put_snapshot(g, v, data))
    }
    fn get_snapshot(&self, g: &str, v: &str) -> io::Result<Vec<u8>> {
        self.execute(|backend| backend.get_snapshot(g, v))
    }
    fn list_snapshots(&self, g: &str) -> io::Result<Vec<String>> {
        self.execute(|backend| backend.list_snapshots(g))
    }
    fn get_index(&self) -> io::Result<Option<Vec<u8>>> {
        self.execute(|backend| backend.get_index())
    }
    fn publish_index(&self, data: &[u8], writer: &str) -> io::Result<()> {
        self.execute(|backend| backend.publish_index(data, writer))
    }
    fn put_index(&self, data: &[u8]) -> io::Result<()> {
        self.execute(|backend| backend.put_index(data))
    }
    fn verify_access(&self) -> io::Result<()> {
        self.execute(|backend| backend.verify_access())
    }
}

#[cfg(test)]
#[path = "../tests/auth/credential_backend.rs"]
mod tests;

struct UnavailableBackend {
    name: &'static str,
    message: String,
}
impl UnavailableBackend {
    fn error<T>(&self) -> io::Result<T> {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            self.message.clone(),
        ))
    }
}
impl Backend for UnavailableBackend {
    fn name(&self) -> &'static str {
        self.name
    }
    fn put_snapshot(&self, _: &str, _: &str, _: &[u8]) -> io::Result<()> {
        self.error()
    }
    fn get_snapshot(&self, _: &str, _: &str) -> io::Result<Vec<u8>> {
        self.error()
    }
    fn list_snapshots(&self, _: &str) -> io::Result<Vec<String>> {
        self.error()
    }
    fn get_index(&self) -> io::Result<Option<Vec<u8>>> {
        self.error()
    }
    fn put_index(&self, _: &[u8]) -> io::Result<()> {
        self.error()
    }
}
