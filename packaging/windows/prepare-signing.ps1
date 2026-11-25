# GitHub Actions only. Never print private key data or place a PFX inside artifacts.
$ErrorActionPreference = 'Stop'
if (!$env:SAVESYNC_SIGNING_PFX_BASE64 -or !$env:SAVESYNC_SIGNING_PFX_PASSWORD) { throw 'Configure SAVESYNC_SIGNING_PFX_BASE64 and SAVESYNC_SIGNING_PFX_PASSWORD repository secrets before creating a release tag' }
$path = Join-Path $env:RUNNER_TEMP 'savesync-signing.pfx'
try {
    [System.IO.File]::WriteAllBytes($path, [Convert]::FromBase64String($env:SAVESYNC_SIGNING_PFX_BASE64))
    $password = ConvertTo-SecureString $env:SAVESYNC_SIGNING_PFX_PASSWORD -AsPlainText -Force
    $certificates = @(Import-PfxCertificate -FilePath $path -CertStoreLocation Cert:\CurrentUser\My -Password $password)
    $signers = @($certificates | Where-Object { $_.HasPrivateKey -and $_.NotAfter -gt (Get-Date) -and $_.EnhancedKeyUsageList.ObjectId -contains '1.3.6.1.5.5.7.3.3' })
    if ($signers.Count -ne 1) { throw 'PFX must contain one unexpired code-signing identity with a private key' }
    $certificate = $signers[0]
    $algorithm = [System.Security.Cryptography.SHA256]::Create()
    try { $fingerprint = ([BitConverter]::ToString($algorithm.ComputeHash($certificate.RawData))).Replace('-', '').ToLowerInvariant() }
    finally { $algorithm.Dispose() }
    "SAVESYNC_SIGNING_THUMBPRINT=$($certificate.Thumbprint)" | Out-File -FilePath $env:GITHUB_ENV -Append -Encoding utf8
    "SAVESYNC_SIGNER_SHA256=$fingerprint" | Out-File -FilePath $env:GITHUB_ENV -Append -Encoding utf8
} finally { if (Test-Path -LiteralPath $path) { Remove-Item -LiteralPath $path -Force } }
