//! Desktop boundary: IPC transport and validation of operation results.
use savesync_core::ipc::{client, Method, Request, ResponsePayload};

pub(crate) trait DaemonClient: Send + Sync {
    fn request(&self, method: Method) -> Result<ResponsePayload, String>;
}

pub(crate) struct IpcDaemonClient;

impl DaemonClient for IpcDaemonClient {
    fn request(&self, method: Method) -> Result<ResponsePayload, String> {
        let response = client::call(&client::socket_name(), Request::new(1, method.clone()))
            .map_err(|error| format!("демон недоступен: {error}"))?;
        if let Some(error) = response.error {
            return Err(error.message);
        }
        let payload = response
            .result
            .ok_or_else(|| "пустой ответ демона".to_string())?;
        validate_result(&method, payload)
    }
}

pub(crate) fn validate_result(
    method: &Method,
    payload: ResponsePayload,
) -> Result<ResponsePayload, String> {
    if let ResponsePayload::Sync(sync) = &payload {
        let failures: Vec<_> = sync
            .results
            .iter()
            .filter(|result| {
                !matches!(
                    result.outcome.as_str(),
                    "uploaded" | "applied" | "up_to_date" | "no_saves"
                )
            })
            .map(|result| {
                format!(
                    "{}: {} {}",
                    result.game_key,
                    result.outcome,
                    result.detail.as_deref().unwrap_or("")
                )
            })
            .collect();
        if !failures.is_empty() {
            return Err(failures.join("\n"));
        }
    }
    let valid = match (method, &payload) {
        (Method::Status, ResponsePayload::Status(_))
        | (Method::Scan | Method::ReloadManifests, ResponsePayload::Scan(_))
        | (Method::History { .. }, ResponsePayload::History(_))
        | (Method::Backup { .. }, ResponsePayload::Backup(_))
        | (Method::GetConfig | Method::SaveConfig { .. }, ResponsePayload::Config(_)) => true,
        (Method::Sync { .. }, ResponsePayload::Sync(_)) => true,
        (Method::Restore { .. }, ResponsePayload::Sync(sync)) => {
            !sync.results.is_empty()
                && sync
                    .results
                    .iter()
                    .all(|result| matches!(result.outcome.as_str(), "applied" | "up_to_date"))
        }
        _ => false,
    };
    if valid {
        Ok(payload)
    } else {
        Err("неожиданный ответ демона".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use savesync_core::ipc::{SyncPayload, SyncResult};
    #[test]
    fn operation_errors_cannot_be_reported_as_success() {
        for outcome in ["error", "locked", "conflict", "skipped"] {
            let payload = ResponsePayload::Sync(SyncPayload {
                results: vec![SyncResult {
                    game_key: "game".into(),
                    outcome: outcome.into(),
                    detail: Some("backend offline".into()),
                }],
            });
            for method in [
                Method::Restore {
                    game_key: "game".into(),
                    version_id: "v1".into(),
                },
                Method::Backup {
                    game_key: "game".into(),
                },
            ] {
                assert!(validate_result(&method, payload.clone())
                    .unwrap_err()
                    .contains(outcome));
            }
        }
    }
    #[test]
    fn empty_restore_and_wrong_payload_are_rejected() {
        let payload = ResponsePayload::Sync(SyncPayload { results: vec![] });
        assert!(validate_result(
            &Method::Restore {
                game_key: "g".into(),
                version_id: "v1".into()
            },
            payload.clone()
        )
        .is_err());
        assert!(validate_result(&Method::GetConfig, payload).is_err());
    }
}
