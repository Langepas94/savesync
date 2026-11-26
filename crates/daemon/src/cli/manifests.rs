use super::*;
pub(crate) fn cmd_manifests(args: &[String]) -> i32 {
    if args.len() != 2 || args[0] != "import" {
        eprintln!("использование: manifests import <папка-базы>");
        return 2;
    }
    let destination = config_path().parent().unwrap().join("manifests");
    match savesync_core::catalog::manifest_store::install(
        std::path::Path::new(&args[1]),
        &destination,
    ) {
        Ok(count) => {
            let socket = savesync_core::ipc::client::socket_name();
            if savesync_core::ipc::client::probe(&socket) {
                match savesync_core::ipc::client::call(
                    &socket,
                    Request::new(1, Method::ReloadManifests),
                ) {
                    Ok(response) if response.error.is_none() => (),
                    _ => {
                        eprintln!("база установлена, но демон не подтвердил обновление; повторите scan после перезапуска");
                        return 1;
                    }
                }
            }
            println!("Установлено манифестов: {count}; custom-правила сохранены");
            0
        }
        Err(error) => {
            eprintln!("обновление базы отклонено: {error}");
            1
        }
    }
}
