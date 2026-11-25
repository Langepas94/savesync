//! Native OAuth: PKCE, loopback callback and refresh-token lifecycle.
use serde::{Deserialize, Serialize};
use std::{io, path::Path};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Google,
    Yandex,
}
impl Provider {
    pub fn parse(value: &str) -> io::Result<Self> {
        match value {
            "google" => Ok(Self::Google),
            "yandex" => Ok(Self::Yandex),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "unknown OAuth provider",
            )),
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Google => "google",
            Self::Yandex => "yandex",
        }
    }
    pub(crate) fn token_endpoint(self) -> &'static str {
        match self {
            Self::Google => "https://oauth2.googleapis.com/token",
            Self::Yandex => "https://oauth.yandex.ru/token",
        }
    }
    pub fn scope(self) -> &'static str {
        match self {
            Self::Google => "https://www.googleapis.com/auth/drive.appdata",
            Self::Yandex => "cloud_api:disk.app_folder",
        }
    }
    fn authorization_endpoint(self) -> &'static str {
        match self {
            Self::Google => "https://accounts.google.com/o/oauth2/v2/auth",
            Self::Yandex => "https://oauth.yandex.ru/authorize",
        }
    }
}

/// Deliberately has no Debug/Serialize implementation containing secret values.
pub struct TokenRecord {
    pub provider: Provider,
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
    pub expires_at: Option<u64>,
    pub scope: Option<String>,
}

#[derive(Deserialize, Default)]
struct Application {
    client_id: String,
    #[serde(default)]
    redirect_port: Option<u16>,
}
#[derive(Deserialize, Default)]
struct Applications {
    google: Option<Application>,
    yandex: Option<Application>,
}
pub struct ClientSettings {
    pub provider: Provider,
    pub client_id: String,
    pub client_secret: Option<String>,
    pub redirect_port: u16,
}
impl ClientSettings {
    pub fn load(profile: &Path, provider: Provider) -> io::Result<Self> {
        let applications = match std::fs::read(profile.join("oauth-apps.json")) {
            Ok(bytes) => serde_json::from_slice::<Applications>(&bytes)?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => Applications::default(),
            Err(error) => return Err(error),
        };
        let name = provider.name().to_ascii_uppercase();
        let application = match provider {
            Provider::Google => applications.google,
            Provider::Yandex => applications.yandex,
        }
        .unwrap_or_default();
        let client_id =
            std::env::var(format!("SAVESYNC_{name}_CLIENT_ID")).unwrap_or(application.client_id);
        if client_id.trim().is_empty() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput,format!("Настройте {provider:?} client_id в oauth-apps.json или SAVESYNC_{name}_CLIENT_ID")));
        }
        Ok(Self {
            provider,
            client_id,
            client_secret: std::env::var(format!("SAVESYNC_{name}_CLIENT_SECRET")).ok(),
            redirect_port: application
                .redirect_port
                .unwrap_or(if provider == Provider::Yandex {
                    17653
                } else {
                    0
                }),
        })
    }
}

pub fn now_seconds() -> u64 {
    crate::snapshot::now_millis() / 1000
}

mod callback;
mod protocol;
pub use callback::AuthorizationSession;
pub(crate) use protocol::access_record;
pub use protocol::{access_token, refresh};
#[cfg(test)]
mod tests;
