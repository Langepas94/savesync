use super::*;
use oauth2::{CsrfToken, PkceCodeChallenge, PkceCodeVerifier};
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use subtle::ConstantTimeEq;

pub struct AuthorizationSession {
    pub authorization_url: String,
    listener: TcpListener,
    settings: ClientSettings,
    state: CsrfToken,
    verifier: PkceCodeVerifier,
    redirect_uri: String,
    token_endpoint: String,
}
impl AuthorizationSession {
    pub fn begin(settings: ClientSettings) -> io::Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", settings.redirect_port))?;
        listener.set_nonblocking(true)?;
        let redirect_uri = format!(
            "http://127.0.0.1:{}/oauth/callback",
            listener.local_addr()?.port()
        );
        let state = CsrfToken::new_random_len(32);
        let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
        let mut url = url::Url::parse(settings.provider.authorization_endpoint())
            .map_err(io::Error::other)?;
        url.query_pairs_mut().extend_pairs([
            ("response_type", "code"),
            ("client_id", settings.client_id.as_str()),
            ("redirect_uri", redirect_uri.as_str()),
            ("state", state.secret().as_str()),
            ("code_challenge", challenge.as_str()),
            ("code_challenge_method", "S256"),
            ("scope", settings.provider.scope()),
        ]);
        if settings.provider == Provider::Google {
            url.query_pairs_mut()
                .extend_pairs([("access_type", "offline"), ("prompt", "consent")]);
        } else {
            url.query_pairs_mut().append_pair("force_confirm", "yes");
        }
        let token_endpoint = settings.provider.token_endpoint().into();
        Ok(Self {
            authorization_url: url.into(),
            listener,
            settings,
            state,
            verifier,
            redirect_uri,
            token_endpoint,
        })
    }

    pub fn complete(self, cancel: Arc<AtomicBool>, timeout: Duration) -> io::Result<TokenRecord> {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline && !cancel.load(Ordering::Relaxed) {
            match self.listener.accept() {
                Ok((mut stream, _)) => {
                    stream.set_read_timeout(Some(Duration::from_secs(3)))?;
                    stream.set_write_timeout(Some(Duration::from_secs(3)))?;
                    let callback = self.read_callback(&mut stream);
                    match callback {
                        Ok(Some(code)) => {
                            let _ = reply(
                                &mut stream,
                                200,
                                "Авторизация получена. Закройте вкладку и вернитесь в SaveSync.",
                            );
                            let mut fields = vec![
                                ("grant_type".into(), "authorization_code".into()),
                                ("code".into(), code),
                                ("client_id".into(), self.settings.client_id.clone()),
                                ("redirect_uri".into(), self.redirect_uri.clone()),
                                ("code_verifier".into(), self.verifier.secret().clone()),
                            ];
                            if let Some(secret) = &self.settings.client_secret {
                                fields.push(("client_secret".into(), secret.clone()));
                            }
                            return super::protocol::token_request(
                                self.settings.provider,
                                &self.token_endpoint,
                                &fields,
                                Some(self.settings.client_id),
                                self.settings.client_secret,
                                None,
                                Some(self.settings.provider.scope().into()),
                            );
                        }
                        Ok(None) => {
                            let _ = reply(&mut stream, 403, "Авторизация отменена.");
                            return Err(io::Error::new(
                                io::ErrorKind::PermissionDenied,
                                "авторизация отклонена пользователем",
                            ));
                        }
                        Err(_) => {
                            let _ = reply(
                                &mut stream,
                                400,
                                "Неверный callback. Вернитесь к странице авторизации SaveSync.",
                            );
                        }
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(40))
                }
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "OAuth отменён или истекло время ожидания",
        ))
    }

    fn read_callback(&self, stream: &mut TcpStream) -> io::Result<Option<String>> {
        let mut line = Vec::new();
        let mut byte = [0u8; 1];
        let deadline = Instant::now() + Duration::from_secs(3);
        while line.len() < 16384 {
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .ok_or_else(|| {
                    io::Error::new(io::ErrorKind::TimedOut, "callback header timed out")
                })?;
            stream.set_read_timeout(Some(remaining.max(Duration::from_millis(1))))?;
            if stream.read(&mut byte)? == 0 {
                break;
            }
            line.push(byte[0]);
            if line.ends_with(b"\r\n\r\n") {
                break;
            }
        }
        let line = std::str::from_utf8(&line)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid callback"))?;
        let parts: Vec<_> = line
            .lines()
            .next()
            .unwrap_or("")
            .split_whitespace()
            .collect();
        if parts.len() != 3 || parts[0] != "GET" || !parts[1].starts_with("/oauth/callback?") {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid callback request",
            ));
        }
        let url =
            url::Url::parse(&format!("http://127.0.0.1{}", parts[1])).map_err(io::Error::other)?;
        let mut parameters = std::collections::HashMap::new();
        for (key, value) in url.query_pairs() {
            if parameters
                .insert(key.into_owned(), value.into_owned())
                .is_some()
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "duplicate callback parameter",
                ));
            }
        }
        let state = parameters
            .get("state")
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing state"))?;
        if state
            .as_bytes()
            .ct_eq(self.state.secret().as_bytes())
            .unwrap_u8()
            != 1
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "OAuth state mismatch",
            ));
        }
        if parameters.contains_key("error") {
            return Ok(None);
        }
        let code = parameters
            .remove("code")
            .filter(|code| !code.is_empty() && code.len() <= 8192)
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "missing authorization code")
            })?;
        Ok(Some(code))
    }
}
fn reply(stream: &mut TcpStream, status: u16, text: &str) -> io::Result<()> {
    let body=format!("<!doctype html><html lang='ru'><meta charset='utf-8'><title>SaveSync</title><p>{text}</p></html>");
    write!(stream,"HTTP/1.1 {status} OAuth\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{body}",body.len())
}

#[cfg(test)]
impl AuthorizationSession {
    pub(super) fn test_endpoint(mut self, endpoint: String) -> Self {
        self.token_endpoint = endpoint;
        self
    }
}
