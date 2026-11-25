//! Stable GitHub release checks and verified Windows installer handoff.
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    io::{self, Read, Write},
    path::{Path, PathBuf},
};
const API: &str = "https://api.github.com/repos/Langepas94/savesync";
const INSTALLER: &str = "savesync-setup.exe";
const MAX_INSTALLER: u64 = 300 * 1024 * 1024;
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
    agent: ureq::Agent,
    token: Option<String>,
}
impl Client {
    fn new() -> Self {
        Self {
            agent: ureq::AgentBuilder::new()
                .timeout(std::time::Duration::from_secs(90))
                .redirects(0)
                .build(),
            token: std::env::var("SAVESYNC_GITHUB_TOKEN").ok(),
        }
    }
    fn api(&self, path: &str, accept: &str) -> ureq::Request {
        let mut request = self
            .agent
            .get(&format!("{API}{path}"))
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
        ureq::Error::Status(code, _) => io::Error::other(format!(
            "release service HTTP {code}; private repositories require SAVESYNC_GITHUB_TOKEN"
        )),
        _ => io::Error::other("release service unavailable"),
    }
}
fn select(bytes: &[u8], current: &str) -> io::Result<Option<Release>> {
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
    let mut installers = metadata
        .assets
        .iter()
        .filter(|asset| asset.name == INSTALLER);
    let installer = installers
        .next()
        .cloned()
        .ok_or_else(|| io::Error::other("signed Windows installer missing"))?;
    if installers.next().is_some() || installer.size == 0 || installer.size > MAX_INSTALLER {
        return Err(io::Error::other("invalid installer assets"));
    }
    let checksums = metadata
        .assets
        .iter()
        .filter(|asset| asset.name == format!("{INSTALLER}.sha256"))
        .cloned()
        .collect::<Vec<_>>();
    if checksums.len() != 1 || checksums[0].size > 4096 {
        return Err(io::Error::other("release checksum missing or ambiguous"));
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
    }))
}
pub fn check() -> io::Result<Option<Release>> {
    let response = Client::new()
        .api("/releases/latest", "application/vnd.github+json")
        .call()
        .map_err(network_error)?;
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 1024 * 1024 {
        return Err(io::Error::other("release metadata too large"));
    }
    select(&bytes, env!("CARGO_PKG_VERSION"))
}
pub fn automatic_updates_configured() -> bool {
    cfg!(windows)
        && option_env!("SAVESYNC_SIGNER_SHA256").is_some_and(|value| {
            value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
        })
}
fn checksum(bytes: &[u8]) -> io::Result<String> {
    let text = std::str::from_utf8(bytes).map_err(io::Error::other)?;
    let fields = text.split_whitespace().collect::<Vec<_>>();
    if fields.len() != 2
        || fields[1] != INSTALLER
        || fields[0].len() != 64
        || !fields[0].bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(io::Error::other("invalid installer checksum file"));
    }
    Ok(fields[0].to_ascii_lowercase())
}
fn download(release: &Release, directory: &Path) -> io::Result<PathBuf> {
    let client = Client::new();
    let mut checksum_bytes = Vec::new();
    client
        .asset(release.checksum.id)?
        .into_reader()
        .take(4097)
        .read_to_end(&mut checksum_bytes)?;
    let expected = checksum(&checksum_bytes)?;
    std::fs::create_dir_all(directory)?;
    let path = directory.join(INSTALLER);
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
            "this build has no trusted Windows update signer; install a signed release",
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
    verify_installer(&installer, &release.version)?;
    let mut command = std::process::Command::new(&installer);
    command.args(["/SILENT", "/NORESTART", "/STARTAFTERINSTALL=1"]);
    command.spawn()?;
    Ok(())
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
#[cfg(not(windows))]
fn verify_installer(_path: &Path, _version: &semver::Version) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "automatic installer is Windows-only; use the SteamOS installer",
    ))
}
#[cfg(test)]
mod tests;
