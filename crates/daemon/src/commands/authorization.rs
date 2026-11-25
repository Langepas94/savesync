//! CLI authorization works locally or delegates its long-lived session to the daemon.
use super::*;
use savesync_core::{
    credentials::{self, OsSecretStore},
    oauth::Provider,
};
fn provider(args: &[String]) -> Result<Provider, String> {
    if args.len() != 1 {
        return Err("remote login/logout <google|yandex>".into());
    }
    Provider::parse(&args[0]).map_err(|error| error.to_string())
}
pub(super) fn save_config(config: Config) -> i32 {
    let socket = savesync_core::ipc::client::socket_name();
    if savesync_core::ipc::client::probe(&socket) {
        return match ipc::call(
            &socket,
            Request::new(
                1,
                Method::SaveConfig {
                    config: Box::new(config),
                },
            ),
        ) {
            Ok(response) => print_response(response),
            Err(error) => {
                eprintln!("демон не подтвердил сохранение: {error}");
                1
            }
        };
    }
    match config.save(&config_path()) {
        Ok(()) => {
            println!("канал сохранён");
            0
        }
        Err(error) => {
            eprintln!("ошибка записи конфига: {error}");
            1
        }
    }
}
pub(super) fn logout(args: &[String]) -> i32 {
    let provider = match provider(args) {
        Ok(value) => value,
        Err(error) => {
            eprintln!("{error}");
            return 2;
        }
    };
    let mut config = match Config::load_or_init(&config_path()) {
        Ok(value) => value,
        Err(error) => {
            eprintln!("config: {error}");
            return 1;
        }
    };
    let removed = match provider {
        Provider::Google => config.backends.google.take().map(|setting| setting.token),
        Provider::Yandex => config.backends.yandex.take().map(|setting| setting.token),
    };
    let result = save_config(config);
    if result == 0 {
        if let Some(reference) = removed.filter(|value| credentials::is_reference(value)) {
            let _ = credentials::delete(&OsSecretStore, &reference);
        }
    }
    result
}
pub(super) fn login(args: &[String]) -> i32 {
    let provider = match provider(args) {
        Ok(value) => value,
        Err(error) => {
            eprintln!("{error}");
            return 2;
        }
    };
    let socket = savesync_core::ipc::client::socket_name();
    let mut runtime = if savesync_core::ipc::client::probe(&socket) {
        None
    } else {
        match load_runtime() {
            Ok(value) => Some(value),
            Err(error) => {
                eprintln!("{error}");
                return 1;
            }
        }
    };
    let mut call = |method: Method| -> Result<ResponsePayload, String> {
        if let Some(runtime) = runtime.as_mut() {
            return Ok(match method {
                Method::OAuthBegin { provider } => runtime.oauth_begin(provider),
                Method::OAuthPoll { provider } => runtime.oauth_poll(provider),
                Method::OAuthCancel { provider } => runtime.oauth_cancel(provider),
                _ => unreachable!(),
            });
        }
        let response =
            ipc::call(&socket, Request::new(1, method)).map_err(|error| error.to_string())?;
        if let Some(error) = response.error {
            return Err(error.message);
        }
        response.result.ok_or_else(|| "пустой ответ демона".into())
    };
    match call(Method::OAuthBegin { provider }) {
        Ok(ResponsePayload::OAuthStart(start)) => println!(
            "Откройте URL в браузере и завершите вход (ожидание до 5 минут):\n{}",
            start.authorization_url
        ),
        Ok(ResponsePayload::OAuthStatus(status)) => {
            eprintln!("{}", status.message.unwrap_or_else(|| "OAuth error".into()));
            return 1;
        }
        Err(error) => {
            eprintln!("{error}");
            return 1;
        }
        _ => return 1,
    }
    loop {
        std::thread::sleep(std::time::Duration::from_millis(250));
        match call(Method::OAuthPoll { provider }) {
            Ok(ResponsePayload::OAuthStatus(status)) if status.state == "pending" => (),
            Ok(ResponsePayload::OAuthStatus(status)) if status.state == "connected" => {
                println!("Облако подключено; токены сохранены в системном хранилище");
                return 0;
            }
            Ok(ResponsePayload::OAuthStatus(status)) => {
                eprintln!(
                    "{}",
                    status.message.unwrap_or_else(|| "Вход отменён".into())
                );
                return 1;
            }
            Err(error) => {
                eprintln!("{error}");
                return 1;
            }
            _ => return 1,
        }
    }
}
