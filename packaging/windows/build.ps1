param([string]$SourceDir = 'target/release', [string]$OutputDir = 'dist', [string]$Version = '0.1.0', [switch]$Signed, [string]$Compiler)
$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$source = (Resolve-Path $SourceDir).Path
if (!(Test-Path -LiteralPath $OutputDir)) { New-Item -ItemType Directory -Path $OutputDir | Out-Null }
$output = (Resolve-Path $OutputDir).Path
if (!$Compiler) {
    $candidates = @("${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe", "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe")
    $Compiler = $candidates | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
}
if (!$Compiler) { throw 'Inno Setup 6 compiler missing; install it from jrsoftware.org' }
foreach ($name in @('savesync-app.exe', 'savesync-daemon.exe')) {
    if (!(Test-Path -LiteralPath (Join-Path $source $name))) { throw "Missing $name" }
}
$arguments = @("/DAppVersion=$Version", "/DSourceDir=$source", "/DOutputDirectory=$output")
if ($Signed) {
    if (!$env:SAVESYNC_SIGNING_THUMBPRINT -or !$env:SAVESYNC_SIGNER_SHA256) { throw 'Signing identity was not prepared' }
    foreach ($name in @('savesync-app.exe', 'savesync-daemon.exe')) { & (Join-Path $PSScriptRoot 'sign.ps1') -Path (Join-Path $source $name) }
    $powershell = (Get-Command pwsh.exe -ErrorAction Stop).Source
    $signScript = Join-Path $PSScriptRoot 'sign.ps1'
    $signCommand = '$q' + $powershell + '$q -NoProfile -NonInteractive -File $q' + $signScript + '$q -Path $f'
    $arguments += '/DSigned'
    $arguments += "/Ssavesync=$signCommand"
}
& $Compiler @arguments (Join-Path $PSScriptRoot 'savesync.iss')
if ($LASTEXITCODE -ne 0) { throw 'Installer compilation failed' }
$name = if ($Signed) { 'savesync-setup.exe' } else { 'savesync-setup-dev.exe' }
$installer = Join-Path $output $name
if ($Signed) {
    & (Get-Command pwsh.exe).Source -NoProfile -NonInteractive -File (Join-Path $repo 'crates/core/src/updates/verify-signer.ps1') -Path $installer -ExpectedSigner $env:SAVESYNC_SIGNER_SHA256 -ExpectedVersion $Version
    if ($LASTEXITCODE -ne 0) { throw 'Final installer signature verification failed' }
}
$hash = (Get-FileHash -LiteralPath $installer -Algorithm SHA256).Hash.ToLowerInvariant()
[System.IO.File]::WriteAllText("$installer.sha256", "$hash  $name`n", [System.Text.UTF8Encoding]::new($false))
Write-Output "Built $installer"
