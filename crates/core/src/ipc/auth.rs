//! Authenticate both ends before any configuration, PIN or mutation request is sent.
use interprocess::local_socket::{prelude::*, Stream};
use std::io;
fn denied() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "IPC peer does not belong to the current user",
    )
}
#[cfg(unix)]
pub fn authorize_peer(stream: &Stream) -> io::Result<()> {
    let peer = stream.peer_creds()?.euid().ok_or_else(denied)?;
    // Root is already privileged and is the normal Decky Loader backend account.
    if allowed_uid(unsafe { libc::geteuid() }, peer) {
        Ok(())
    } else {
        Err(denied())
    }
}
#[cfg(unix)]
fn allowed_uid(current: u32, peer: u32) -> bool {
    peer == current || peer == 0
}
#[cfg(windows)]
pub fn authorize_peer(stream: &Stream) -> io::Result<()> {
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    let pid = stream.peer_creds()?.pid().ok_or_else(denied)?;
    unsafe {
        let process = Handle(OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid));
        if process.0.is_null() {
            return Err(denied());
        }
        if user_sid(process.0)? == user_sid(GetCurrentProcess())? {
            Ok(())
        } else {
            Err(denied())
        }
    }
}
#[cfg(windows)]
struct Handle(windows_sys::Win32::Foundation::HANDLE);
#[cfg(windows)]
impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                windows_sys::Win32::Foundation::CloseHandle(self.0);
            }
        }
    }
}
#[cfg(windows)]
unsafe fn user_sid(process: windows_sys::Win32::Foundation::HANDLE) -> io::Result<Vec<u8>> {
    use windows_sys::Win32::{
        Security::{
            CopySid, GetLengthSid, GetTokenInformation, TokenUser, TOKEN_QUERY, TOKEN_USER,
        },
        System::Threading::OpenProcessToken,
    };
    let mut token = Handle(std::ptr::null_mut());
    if OpenProcessToken(process, TOKEN_QUERY, &mut token.0) == 0 {
        return Err(denied());
    }
    let mut length = 0;
    GetTokenInformation(token.0, TokenUser, std::ptr::null_mut(), 0, &mut length);
    if length == 0 {
        return Err(denied());
    }
    // usize backing storage provides alignment for TOKEN_USER's native pointer.
    let mut storage = vec![0usize; (length as usize).div_ceil(std::mem::size_of::<usize>())];
    if GetTokenInformation(
        token.0,
        TokenUser,
        storage.as_mut_ptr().cast(),
        length,
        &mut length,
    ) == 0
    {
        return Err(denied());
    }
    let user = &*storage.as_ptr().cast::<TOKEN_USER>();
    let length = GetLengthSid(user.User.Sid);
    if length == 0 {
        return Err(denied());
    }
    let mut sid = vec![0u8; length as usize];
    if CopySid(length, sid.as_mut_ptr().cast(), user.User.Sid) == 0 {
        return Err(denied());
    }
    Ok(sid)
}
#[cfg(test)]
mod tests {
    #[cfg(unix)]
    #[test]
    fn other_unprivileged_users_are_rejected() {
        assert!(super::allowed_uid(1000, 1000));
        assert!(super::allowed_uid(1000, 0));
        assert!(!super::allowed_uid(1000, 1001));
    }
    #[cfg(windows)]
    #[test]
    fn current_process_has_stable_token_identity() {
        unsafe {
            let process = windows_sys::Win32::System::Threading::GetCurrentProcess();
            assert_eq!(
                super::user_sid(process).unwrap(),
                super::user_sid(process).unwrap()
            );
        }
    }
}
