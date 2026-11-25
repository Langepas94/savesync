use super::*;
pub(crate) fn cmd_serve() -> i32 {
    let mut runtime = match load_runtime() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("ошибка: {e}");
            return 1;
        }
    };
    if let Err(e) = runtime.start_lan() {
        eprintln!("LAN-сервер: {e}");
        return 1;
    }
    let runtime = Arc::new(Mutex::new(runtime));
    // Авто-бэкапы по расписанию (plan §2.2): проверка раз в 30 минут.
    {
        let rt = runtime.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(std::time::Duration::from_secs(30 * 60));
            let mut rt = match rt.lock() {
                Ok(rt) => rt,
                Err(_) => continue,
            };
            if rt.shutdown_requested {
                break;
            }
            if rt.auto_backup_due() {
                match rt.run_auto_backup() {
                    Ok(n) => {
                        if n > 0 {
                            println!("авто-бэкап: {n} игр");
                        }
                        rt.mark_auto_backup_done();
                    }
                    Err(e) => eprintln!("авто-бэкап: {e}"),
                }
            }
        });
    }
    match ipc::serve(runtime, &savesync_core::ipc::client::socket_name()) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("ошибка IPC-сервера: {e}");
            1
        }
    }
}
