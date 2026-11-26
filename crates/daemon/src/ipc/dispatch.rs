//! Диспетчеризация IPC и общая адаптация операций для офлайн CLI.

use std::sync::{Arc, Mutex, RwLock, TryLockError};

use savesync_core::ipc::{
    ConfigPayload, Method, Request, Response, ResponsePayload, StatusPayload, SyncPayload,
    SyncResult,
};

use crate::service::{AddGameError, Runtime};

pub(super) fn dispatch(
    req: Request,
    runtime: &Arc<Mutex<Runtime>>,
    cache: &RwLock<StatusPayload>,
) -> Response {
    if matches!(req.method, Method::Status) {
        return match runtime.try_lock() {
            Ok(runtime) => {
                if runtime.shutdown_requested {
                    return Response::err(req.id, "daemon is stopping");
                }
                match cache.read() {
                    Ok(value) => {
                        let mut snapshot = value.clone();
                        snapshot.last_backup = runtime.last_backup();
                        Response::ok(req.id, ResponsePayload::Status(snapshot))
                    }
                    Err(_) => Response::err(req.id, "status cache unavailable"),
                }
            }
            Err(TryLockError::WouldBlock) => match cache.read() {
                Ok(value) => {
                    let mut status = value.clone();
                    status.busy = true;
                    Response::ok(req.id, ResponsePayload::Status(status))
                }
                Err(_) => Response::err(req.id, "status cache unavailable"),
            },
            Err(TryLockError::Poisoned(_)) => Response::err(req.id, "daemon state poisoned"),
        };
    }
    if matches!(req.method, Method::LanPresence) {
        let directory = match runtime.lock() {
            Ok(runtime) if !runtime.shutdown_requested => runtime.lan_presence_directory(),
            Ok(_) => return Response::err(req.id, "daemon is stopping"),
            Err(_) => return Response::err(req.id, "daemon state poisoned"),
        };
        return match crate::service::lan_presence(directory) {
            Ok(online) => Response::ok(req.id, ResponsePayload::LanPresence(online)),
            Err(error) => Response::err(req.id, error.to_string()),
        };
    }
    if let Method::CloudCheck { provider } = req.method {
        let reference = match runtime.lock() {
            Ok(runtime) if !runtime.shutdown_requested => runtime.cloud_reference(provider),
            Ok(_) => return Response::err(req.id, "daemon is stopping"),
            Err(_) => return Response::err(req.id, "daemon state poisoned"),
        };
        return Response::ok(
            req.id,
            ResponsePayload::OAuthStatus(crate::service::cloud_check(provider, reference)),
        );
    }
    let mut rt = match runtime.lock() {
        Ok(rt) => rt,
        Err(_) => return Response::err(req.id, "daemon state poisoned"),
    };
    if rt.shutdown_requested {
        return Response::err(req.id, "daemon is stopping");
    }
    let payload = match execute(&mut rt, &req.method) {
        Ok(payload) => payload,
        Err(error) => return Response::err(req.id, error),
    };
    if !rt.shutdown_requested {
        let status = rt.status();
        if let Ok(mut value) = cache.write() {
            *value = status;
        }
    }
    Response::ok(req.id, payload)
}

pub(crate) fn execute(rt: &mut Runtime, method: &Method) -> Result<ResponsePayload, String> {
    let payload = match method {
        Method::Shutdown => {
            rt.prepare_shutdown();
            ResponsePayload::Stopped
        }
        Method::Status => ResponsePayload::Status(rt.status()),
        Method::Scan => ResponsePayload::Scan(rt.scan()),
        Method::ReloadManifests => match rt.reload_manifests() {
            Ok(scan) => ResponsePayload::Scan(scan),
            Err(error) => operation_error("manifests", error.to_string()),
        },
        Method::AddCustomGame { name, paths } => match rt.add_custom_game(name, paths) {
            Ok(scan) => ResponsePayload::Scan(scan),
            Err(AddGameError::Invalid(message)) => return Err(message),
            Err(AddGameError::Visibility { game_key, message }) => {
                operation_error(&game_key, message)
            }
        },
        Method::SetGameHidden { game_key, hidden } => match rt.set_game_hidden(game_key, *hidden) {
            Ok(scan) => ResponsePayload::Scan(scan),
            Err(error) => operation_error(game_key, error),
        },
        Method::Lan { action } => ResponsePayload::Lan(rt.lan_action(action.clone())),
        Method::OAuthBegin {
            provider,
            client_id,
        } => match rt.oauth_begin(*provider, client_id.as_deref()) {
            Ok(start) => ResponsePayload::OAuthStart(start),
            Err(status) => ResponsePayload::OAuthStatus(status),
        },
        Method::OAuthPoll { provider } => ResponsePayload::OAuthStatus(rt.oauth_poll(*provider)),
        Method::OAuthCancel { provider } => {
            ResponsePayload::OAuthStatus(rt.oauth_cancel(*provider))
        }
        Method::Sync { game_key } => ResponsePayload::Sync(rt.sync(game_key.as_deref())),
        Method::History { game_key } => ResponsePayload::History(rt.history(game_key)),
        Method::Restore {
            game_key,
            version_id,
        } => ResponsePayload::Sync(rt.restore(game_key, version_id)),
        Method::Backup { game_key } => match rt.backup_game(game_key) {
            Ok(backup) => ResponsePayload::Backup(backup),
            Err(error) => operation_error(game_key, error),
        },
        Method::BackupAll => ResponsePayload::BackupBatch(rt.backup_all()),
        Method::GetConfig => config_payload(rt),
        Method::SaveConfig { config } => match rt.save_config(*config.clone()) {
            Ok(()) => config_payload(rt),
            Err(error) => operation_error("config", error.to_string()),
        },
        Method::LanPresence | Method::CloudCheck { .. } => {
            unreachable!("operations that release the runtime lock are handled at the entry point")
        }
    };
    Ok(payload)
}

fn config_payload(runtime: &Runtime) -> ResponsePayload {
    ResponsePayload::Config(ConfigPayload {
        config: runtime.config.clone(),
    })
}

fn operation_error(key: &str, message: String) -> ResponsePayload {
    ResponsePayload::Sync(SyncPayload {
        results: vec![SyncResult {
            game_key: key.into(),
            outcome: "error".into(),
            detail: Some(message),
        }],
    })
}
