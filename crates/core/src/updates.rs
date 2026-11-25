//! Stable GitHub release checks and verified Windows installer handoff.
use crate::credentials::{OsSecretStore, SecretStore};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    io::{self, Read, Write},
    path::{Path, PathBuf},
};
const API: &str = "https://api.github.com/repos/Langepas94/savesync";
const INSTALLER: &str = "savesync-setup.exe";
const MAX_INSTALLER: u64 = 300 * 1024 * 1024;
const GITHUB_TOKEN_KEY: &str = "github-release-token";

pub fn save_github_token(token: String) -> io::Result<()> {
    let token = zeroize::Zeroizing::new(token);
    let trimmed = token.trim();
    if trimmed.is_empty() || !trimmed.bytes().all(|byte| byte.is_ascii_graphic()) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Введите токен GitHub",
        ));
    }
    OsSecretStore.put(GITHUB_TOKEN_KEY, trimmed)
}

pub fn remove_github_token() -> io::Result<()> {
    match OsSecretStore.delete(GITHUB_TOKEN_KEY) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}
#[derive(Debug, Clone, Deserialize)]
struct Asset {
    id: u64,
    name: String,
    size: u64,
}
#[derive(Debug, Clone)]
pub struct Release {
    pub version: semver::Version,
    pub page_url: String,
    installer: Asset,
    checksum: Asset,
    signature: Option<Asset>,
}

pub enum ReleaseCheck {
    NoPublishedRelease,
    UpToDate,
    Available(Release),
}
#[derive(Deserialize)]
struct Metadata {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    html_url: String,
    assets: Vec<Asset>,
}
struct Client {
    api_base: String,
    agent: ureq::Agent,
    token: Option<String>,
}
impl Client {
    fn new() -> Self {
        Self {
            api_base: API.into(),
            agent: ureq::AgentBuilder::new()
                .timeout(std::time::Duration::from_secs(90))
                .redirects(0)
                .build(),
            token: std::env::var("SAVESYNC_GITHUB_TOKEN")
                .ok()
                .map(|token| token.trim().to_owned())
                .filter(|token| !token.is_empty())
                .or_else(|| OsSecretStore.get(GITHUB_TOKEN_KEY).ok()),
        }
    }
    fn api(&self, path: &str, accept: &str) -> ureq::Request {
        let mut request = self
            .agent
            .get(&format!("{}{path}", self.api_base))
            .set("User-Agent", "SaveSync")
            .set("Accept", accept)
            .set("X-GitHub-Api-Version", "2022-11-28");
        if let Some(token) = &self.token {
            request = request.set("Authorization", &format!("Bearer {token}"));
        }
        request
    }
    fn asset(&self, id: u64) -> io::Result<ureq::Response> {
        let response = self
            .api(
                &format!("/releases/assets/{id}"),
                "application/octet-stream",
            )
            .call()
            .map_err(network_error)?;
        if (300..400).contains(&response.status()) {
            let location = response
                .header("Location")
                .ok_or_else(|| io::Error::other("release redirect missing"))?;
            let url = url::Url::parse(location).map_err(io::Error::other)?;
            if url.scheme() != "https"
                || !matches!(
                    url.host_str(),
                    Some(
                        "release-assets.githubusercontent.com"
                            | "objects.githubusercontent.com"
                            | "github.com"
                    )
                )
            {
                return Err(io::Error::other("unexpected release download origin"));
            }
            // The API token is never forwarded to the signed asset URL.
            return self.agent.get(location).call().map_err(network_error);
        }
        Ok(response)
    }
}
fn network_error(error: ureq::Error) -> io::Error {
    match error {
        ureq::Error::Status(code, _) => {
            io::Error::other(format!("GitHub Releases вернул HTTP {code}"))
        }
        _ => io::Error::other("GitHub Releases недоступен"),
    }
}
fn select_for(bytes: &[u8], current: &str, package: &str) -> io::Result<Option<Release>> {
    let metadata: Metadata = serde_json::from_slice(bytes).map_err(io::Error::other)?;
    if metadata.draft || metadata.prerelease {
        return Ok(None);
    }
    let version = semver::Version::parse(
        metadata
            .tag_name
            .strip_prefix('v')
            .unwrap_or(&metadata.tag_name),
    )
    .map_err(io::Error::other)?;
    if !version.pre.is_empty()
        || version <= semver::Version::parse(current).map_err(io::Error::other)?
    {
        return Ok(None);
    }
    let mut installers = metadata.assets.iter().filter(|asset| asset.name == package);
    let Some(installer) = installers.next().cloned() else {
        return Ok(None);
    };
    if installers.next().is_some() || installer.size == 0 || installer.size > MAX_INSTALLER {
        return Err(io::Error::other("invalid installer assets"));
    }
    let checksums = metadata
        .assets
        .iter()
        .filter(|asset| asset.name == format!("{package}.sha256"))
        .cloned()
        .collect::<Vec<_>>();
    if checksums.len() != 1 || checksums[0].size > 4096 {
        return Err(io::Error::other("release checksum missing or ambiguous"));
    }
    let signatures: Vec<_> = metadata
        .assets
        .iter()
        .filter(|asset| asset.name == format!("{package}.sha256.sig"))
        .cloned()
        .collect();
    if package != INSTALLER && (signatures.len() != 1 || signatures[0].size != 64) {
        return Err(io::Error::other("release signature missing or ambiguous"));
    }
    let url = url::Url::parse(&metadata.html_url).map_err(io::Error::other)?;
    if url.scheme() != "https"
        || url.host_str() != Some("github.com")
        || !url.path().starts_with("/Langepas94/savesync/releases/")
    {
        return Err(io::Error::other("unexpected release page"));
    }
    Ok(Some(Release {
        version,
        page_url: metadata.html_url,
        installer,
        checksum: checksums[0].clone(),
        signature: signatures.into_iter().next(),
    }))
}
fn select(bytes: &[u8], current: &str) -> io::Result<Option<Release>> {
    select_for(
        bytes,
        current,
        if cfg!(windows) {
            INSTALLER
        } else {
            "savesync-steamos.tar.gz"
        },
    )
}
pub fn check() -> io::Result<ReleaseCheck> {
    check_with(&Client::new())
}

fn check_with(client: &Client) -> io::Result<ReleaseCheck> {
    let accept = "application/vnd.github+json";
    let response = match client.api("/releases/latest", accept).call() {
        Ok(response) => response,
        Err(ureq::Error::Status(404, _)) => {
            // GitHub uses 404 for both a hidden repository and a missing stable release.
            return match client.api("", accept).call() {
                Ok(_) => Ok(ReleaseCheck::NoPublishedRelease),
                Err(ureq::Error::Status(404, _)) if client.token.is_none() => Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "Репозиторий обновлений недоступен. Добавьте токен в разделе «Доступ к приватным релизам».",
                )),
                Err(ureq::Error::Status(404, _)) => Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "Репозиторий обновлений недоступен. Проверьте доступ токена к репозиторию SaveSync.",
                )),
                Err(error) => Err(network_error(error)),
            };
        }
        Err(error) => return Err(network_error(error)),
    };
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 1024 * 1024 {
        return Err(io::Error::other("release metadata too large"));
    }
    Ok(match select(&bytes, env!("CARGO_PKG_VERSION"))? {
        Some(release) => ReleaseCheck::Available(release),
        None => ReleaseCheck::UpToDate,
    })
}
pub fn automatic_updates_configured() -> bool {
    (cfg!(windows)
        && option_env!("SAVESYNC_SIGNER_SHA256").is_some_and(|value| {
            value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
        }))
        || (cfg!(target_os = "linux") && update_key().is_ok())
}
fn checksum_for(bytes: &[u8], package: &str) -> io::Result<String> {
    let text = std::str::from_utf8(bytes).map_err(io::Error::other)?;
    let fields = text.split_whitespace().collect::<Vec<_>>();
    if fields.len() != 2
        || fields[1] != package
        || fields[0].len() != 64
        || !fields[0].bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(io::Error::other("invalid installer checksum file"));
    }
    Ok(fields[0].to_ascii_lowercase())
}
#[cfg(test)]
fn checksum(bytes: &[u8]) -> io::Result<String> {
    checksum_for(bytes, INSTALLER)
}
fn update_key() -> io::Result<ed25519_dalek::VerifyingKey> {
    let hex = option_env!("SAVESYNC_UPDATE_PUBLIC_KEY").unwrap_or("");
    if hex.len() != 64 || !hex.is_ascii() {
        return Err(io::Error::other("update public key not configured"));
    }
    let mut key = [0u8; 32];
    for (i, byte) in key.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).map_err(io::Error::other)?;
    }
    ed25519_dalek::VerifyingKey::from_bytes(&key).map_err(io::Error::other)
}
fn verify_checksum_signature(
    key: &ed25519_dalek::VerifyingKey,
    bytes: &[u8],
    signature: &[u8],
) -> io::Result<()> {
    let sig = ed25519_dalek::Signature::from_slice(signature).map_err(io::Error::other)?;
    key.verify_strict(bytes, &sig).map_err(io::Error::other)
}
fn download(release: &Release, directory: &Path) -> io::Result<PathBuf> {
    let client = Client::new();
    let mut checksum_bytes = Vec::new();
    client
        .asset(release.checksum.id)?
        .into_reader()
        .take(4097)
        .read_to_end(&mut checksum_bytes)?;
    if let Some(signature) = &release.signature {
        let mut bytes = Vec::new();
        client
            .asset(signature.id)?
            .into_reader()
            .take(65)
            .read_to_end(&mut bytes)?;
        verify_checksum_signature(&update_key()?, &checksum_bytes, &bytes)?;
    }
    let expected = checksum_for(&checksum_bytes, &release.installer.name)?;
    std::fs::create_dir_all(directory)?;
    let path = directory.join(&release.installer.name);
    let mut file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&path)?;
    let result = (|| {
        let mut reader = client
            .asset(release.installer.id)?
            .into_reader()
            .take(release.installer.size + 1);
        let mut hash = Sha256::new();
        let mut total = 0;
        let mut buffer = [0; 65536];
        loop {
            let count = reader.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            total += count as u64;
            if total > release.installer.size {
                return Err(io::Error::other("installer exceeded advertised size"));
            }
            hash.update(&buffer[..count]);
            file.write_all(&buffer[..count])?;
        }
        if total != release.installer.size || format!("{:x}", hash.finalize()) != expected {
            return Err(io::Error::other("installer checksum/size mismatch"));
        }
        file.sync_all()?;
        Ok(())
    })();
    drop(file);
    if let Err(error) = result {
        let _ = std::fs::remove_file(&path);
        return Err(error);
    }
    Ok(path)
}
/// Called only after the user selects installation in the app.
pub fn install(release: &Release) -> io::Result<()> {
    if !automatic_updates_configured() {
        return Err(io::Error::other(
            "this build has no trusted update signing key; install the release manually",
        ));
    }
    let directory = crate::config::data_dir(crate::paths::Platform::current())
        .ok_or_else(|| io::Error::other("profile unavailable"))?
        .join("updates")
        .join(format!(
            "{}-{}",
            release.version,
            blake3::hash(oauth2::CsrfToken::new_random().secret().as_bytes()).to_hex()
        ));
    let installer = download(release, &directory)?;
    launch_update(&installer, &release.version)?;
    Ok(())
}
#[cfg(windows)]
fn launch_update(installer: &Path, version: &semver::Version) -> io::Result<()> {
    verify_installer(installer, version)?;
    std::process::Command::new(installer)
        .args(["/SILENT", "/NORESTART", "/STARTAFTERINSTALL=1"])
        .spawn()?;
    Ok(())
}
#[cfg(target_os = "linux")]
fn launch_update(package: &Path, version: &semver::Version) -> io::Result<()> {
    let directory = package
        .parent()
        .ok_or_else(|| io::Error::other("update directory missing"))?;
    let status = std::process::Command::new("tar")
        .args(["-xzf"])
        .arg(package)
        .arg("-C")
        .arg(directory)
        .status()?;
    if !status.success() {
        return Err(io::Error::other("update package extraction failed"));
    }
    let root = directory.join("savesync-steamos");
    if std::fs::read_to_string(root.join("VERSION"))?.trim() != version.to_string() {
        return Err(io::Error::other("update version mismatch"));
    }
    let script = root.join("install-steamos.sh");
    let installed_app = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| io::Error::other("home directory unavailable"))?
        .join(".local/bin/savesync-app");
    if std::env::current_exe()?.canonicalize()? != installed_app.canonicalize()? {
        return Err(io::Error::other(
            "automatic update requires the user-installed SaveSync app",
        ));
    }
    std::process::Command::new("bash")
        .args(["-c", "bash \"$1\"; exec \"$2\"", "savesync-update"])
        .arg(&script)
        .arg(installed_app)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;
    Ok(())
}
#[cfg(not(any(windows, target_os = "linux")))]
fn launch_update(_: &Path, _: &semver::Version) -> io::Result<()> {
    Err(io::Error::other("unsupported update platform"))
}
#[cfg(windows)]
fn verify_installer(path: &Path, version: &semver::Version) -> io::Result<()> {
    use std::os::windows::process::CommandExt;
    let script = path.with_file_name("verify-signer.ps1");
    std::fs::write(&script, include_str!("updates/verify-signer.ps1"))?;
    let powershell = std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("C:\\Windows"))
        .join("System32/WindowsPowerShell/v1.0/powershell.exe");
    let status = std::process::Command::new(powershell)
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(&script)
        .arg("-Path")
        .arg(path)
        .arg("-ExpectedSigner")
        .arg(option_env!("SAVESYNC_SIGNER_SHA256").unwrap_or(""))
        .arg("-ExpectedVersion")
        .arg(format!(
            "{}.{}.{}",
            version.major, version.minor, version.patch
        ))
        .creation_flags(0x08000000)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()?;
    let _ = std::fs::remove_file(script);
    if !status.success() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "installer signature, publisher or version verification failed",
        ));
    }
    Ok(())
}
#[cfg(test)]
mod tests;
