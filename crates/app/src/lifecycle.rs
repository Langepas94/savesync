//! User-context desktop lifecycle: one app instance and a detached daemon child.
use savesync_core::{config::data_dir, ipc::client, paths::Platform};
use std::{
    fs::OpenOptions,
    io,
    process::{Command, Stdio},
};
pub(crate) fn instance() -> io::Result<std::fs::File> {
    let directory =
        data_dir(Platform::current()).ok_or_else(|| io::Error::other("profile unavailable"))?;
    std::fs::create_dir_all(&directory)?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(directory.join("app.lock"))?;
    fs2::FileExt::try_lock_exclusive(&lock)?;
    Ok(lock)
}
pub(crate) fn start_daemon() -> io::Result<()> {
    if client::probe(&client::socket_name()) {
        return Ok(());
    }
    let mut path = std::env::current_exe()?;
    path.set_file_name(if cfg!(windows) {
        "savesync-daemon.exe"
    } else {
        "savesync-daemon"
    });
    if !path.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "daemon executable missing; install or build the complete package",
        ));
    }
    let mut command = Command::new(path);
    command
        .arg("serve")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    command.spawn()?;
    Ok(())
}
pub(crate) fn activate_existing() {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            FindWindowW, SetForegroundWindow, ShowWindow, SW_RESTORE,
        };
        let title = "SaveSync\0".encode_utf16().collect::<Vec<_>>();
        let window = FindWindowW(std::ptr::null(), title.as_ptr());
        if !window.is_null() {
            ShowWindow(window, SW_RESTORE);
            SetForegroundWindow(window);
        }
    }
}

fn exit_path() -> io::Result<std::path::PathBuf> {
    Ok(data_dir(Platform::current())
        .ok_or_else(|| io::Error::other("profile unavailable"))?
        .join("app-exit.request"))
}
pub(crate) fn consume_exit_request() -> bool {
    exit_path().is_ok_and(|path| path.is_file() && std::fs::remove_file(path).is_ok())
}
pub(crate) fn request_exit() -> io::Result<()> {
    let path = exit_path()?;
    if let Ok(_guard) = instance() {
        let _ = std::fs::remove_file(path);
        return Ok(());
    }
    std::fs::write(&path, b"exit")?;
    // A hidden window may stop receiving redraws until it becomes visible.
    // Wake it so the running app can consume the exit request.
    activate_existing();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while std::time::Instant::now() < deadline {
        if let Ok(_guard) = instance() {
            let _ = std::fs::remove_file(path);
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        "SaveSync app did not exit; installation cancelled",
    ))
}
