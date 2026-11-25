//! OAuth jobs never hold Runtime's mutex while waiting for browser consent.
use super::*;
use savesync_core::{
    credentials::{self, OsSecretStore},
    ipc::{OAuthStartPayload, OAuthStatusPayload},
    oauth::{AuthorizationSession, ClientSettings, Provider},
};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Receiver, TryRecvError},
    Arc,
};

pub struct OAuthJob {
    result: Receiver<Result<String, String>>,
    cancel: Arc<AtomicBool>,
}
impl Drop for OAuthJob {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        if let Ok(Ok(reference)) = self.result.try_recv() {
            let _ = credentials::delete(&OsSecretStore, &reference);
        }
    }
}
fn status(provider: Provider, state: &str, message: Option<String>) -> ResponsePayload {
    ResponsePayload::OAuthStatus(OAuthStatusPayload {
        provider,
        state: state.into(),
        message,
    })
}

impl Runtime {
    pub fn oauth_begin(&mut self, provider: Provider) -> ResponsePayload {
        let profile = self
            .state_dir
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."));
        let session =
            match ClientSettings::load(profile, provider).and_then(AuthorizationSession::begin) {
                Ok(session) => session,
                Err(error) => return status(provider, "error", Some(error.to_string())),
            };
        let authorization_url = session.authorization_url.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let signal = cancel.clone();
        let (sender, result) = mpsc::channel();
        std::thread::spawn(move || {
            let outcome = session
                .complete(signal, std::time::Duration::from_secs(300))
                .and_then(|record| credentials::save(&OsSecretStore, None, &record))
                .map_err(|error| error.to_string());
            if let Err(mpsc::SendError(Ok(reference))) = sender.send(outcome) {
                let _ = credentials::delete(&OsSecretStore, &reference);
            }
        });
        self.oauth_jobs
            .insert(provider, OAuthJob { result, cancel });
        ResponsePayload::OAuthStart(OAuthStartPayload {
            provider,
            authorization_url,
        })
    }

    pub fn oauth_poll(&mut self, provider: Provider) -> ResponsePayload {
        let result = match self.oauth_jobs.get(&provider) {
            Some(job) => match job.result.try_recv() {
                Ok(result) => result,
                Err(TryRecvError::Empty) => return status(provider, "pending", None),
                Err(TryRecvError::Disconnected) => {
                    Err("OAuth job завершился без результата".into())
                }
            },
            None => return status(provider, "idle", None),
        };
        self.oauth_jobs.remove(&provider);
        match result {
            Ok(reference) => {
                let mut config = self.config.clone();
                match provider {
                    Provider::Google => {
                        config.backends.google = Some(savesync_core::config::GoogleConfig {
                            token: reference.clone(),
                        })
                    }
                    Provider::Yandex => {
                        config.backends.yandex = Some(savesync_core::config::YandexConfig {
                            token: reference.clone(),
                        })
                    }
                }
                let path = self.state_dir.parent().unwrap().join("config.json");
                match config.save(&path) {
                    Ok(()) => {
                        self.config = config;
                        status(provider, "connected", None)
                    }
                    Err(error) => {
                        let _ = credentials::delete(&OsSecretStore, &reference);
                        status(provider, "error", Some(error.to_string()))
                    }
                }
            }
            Err(error) => status(provider, "error", Some(error)),
        }
    }

    pub fn oauth_cancel(&mut self, provider: Provider) -> ResponsePayload {
        self.oauth_jobs.remove(&provider);
        status(provider, "idle", Some("Вход отменён".into()))
    }
}
