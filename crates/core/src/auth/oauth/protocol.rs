use super::*;
use crate::auth::credentials::{self, OsSecretStore, SecretStore};
use std::time::Duration;

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    token_type: String,
    refresh_token: Option<String>,
    expires_in: Option<u64>,
    scope: Option<String>,
}

pub(super) fn token_request(
    provider: Provider,
    endpoint: &str,
    fields: &[(String, String)],
    client_id: Option<String>,
    client_secret: Option<String>,
    fallback_refresh: Option<String>,
    fallback_scope: Option<String>,
) -> io::Result<TokenRecord> {
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(30))
        .redirects(0)
        .build();
    let fields: Vec<_> = fields
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect();
    let response = agent
        .post(endpoint)
        .send_form(&fields)
        .map_err(|error| match error {
            ureq::Error::Status(status, _) => io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("OAuth token request rejected (HTTP {status}); повторите вход"),
            ),
            _ => io::Error::new(
                io::ErrorKind::ConnectionAborted,
                "OAuth token service unavailable",
            ),
        })?;
    let token: TokenResponse = response
        .into_json()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid OAuth token response"))?;
    if token.access_token.is_empty()
        || token
            .access_token
            .bytes()
            .any(|byte| byte <= 32 || byte >= 127)
        || !token.token_type.eq_ignore_ascii_case("bearer")
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid OAuth access token",
        ));
    }
    let scope = token.scope.or(fallback_scope);
    if scope.as_ref().is_some_and(|scope| {
        !scope
            .split_whitespace()
            .any(|granted| granted == provider.scope())
    }) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "OAuth did not grant the required save-sync scope",
        ));
    }
    Ok(TokenRecord {
        provider,
        access_token: token.access_token,
        refresh_token: token.refresh_token.or(fallback_refresh),
        client_id,
        client_secret,
        expires_at: token
            .expires_in
            .map(|seconds| now_seconds().saturating_add(seconds)),
        scope,
    })
}

pub fn refresh(record: &TokenRecord) -> io::Result<TokenRecord> {
    refresh_at(record, record.provider.token_endpoint())
}
pub(super) fn refresh_at(record: &TokenRecord, endpoint: &str) -> io::Result<TokenRecord> {
    let refresh = record.refresh_token.as_deref().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::PermissionDenied,
            "refresh token unavailable; повторите вход",
        )
    })?;
    let client_id = record.client_id.as_deref().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::PermissionDenied,
            "OAuth client ID unavailable; повторите вход",
        )
    })?;
    let mut fields = vec![
        ("grant_type".into(), "refresh_token".into()),
        ("refresh_token".into(), refresh.into()),
        ("client_id".into(), client_id.into()),
    ];
    if let Some(secret) = &record.client_secret {
        fields.push(("client_secret".into(), secret.clone()));
    }
    token_request(
        record.provider,
        endpoint,
        &fields,
        record.client_id.clone(),
        record.client_secret.clone(),
        record.refresh_token.clone(),
        record.scope.clone(),
    )
}

pub fn access_token(
    reference: &str,
    provider: Provider,
    force_refresh: bool,
) -> io::Result<String> {
    access_record(reference, provider, force_refresh).map(|record| record.access_token)
}
pub(crate) fn access_record(
    reference: &str,
    provider: Provider,
    force_refresh: bool,
) -> io::Result<TokenRecord> {
    let state = crate::config::data_dir(crate::platform::paths::Platform::current())
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "profile directory unavailable"))?
        .join("state");
    access_record_with(
        reference,
        provider,
        force_refresh,
        &OsSecretStore,
        provider.token_endpoint(),
        &state,
    )
}
#[cfg(test)]
pub(crate) fn access_token_with(
    reference: &str,
    provider: Provider,
    force_refresh: bool,
    store: &dyn SecretStore,
    endpoint: &str,
    state: &Path,
) -> io::Result<String> {
    access_record_with(reference, provider, force_refresh, store, endpoint, state)
        .map(|record| record.access_token)
}
pub(crate) fn access_record_with(
    reference: &str,
    provider: Provider,
    force_refresh: bool,
    store: &dyn SecretStore,
    endpoint: &str,
    state: &Path,
) -> io::Result<TokenRecord> {
    if !credentials::is_reference(reference) {
        return Ok(TokenRecord {
            provider,
            access_token: reference.into(),
            refresh_token: None,
            client_id: None,
            client_secret: None,
            expires_at: None,
            scope: None,
        });
    }
    let directory = state.join("oauth-locks");
    std::fs::create_dir_all(&directory)?;
    let key = blake3::hash(reference.as_bytes()).to_hex().to_string();
    let guard = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(directory.join(key))?;
    fs2::FileExt::lock_exclusive(&guard)?;
    let mut record = credentials::load(store, reference, provider)?;
    if force_refresh
        || record
            .expires_at
            .is_some_and(|expiry| expiry <= now_seconds().saturating_add(60))
    {
        record = refresh_at(&record, endpoint)?;
        credentials::save(store, Some(reference), &record)?;
    }
    Ok(record)
}
