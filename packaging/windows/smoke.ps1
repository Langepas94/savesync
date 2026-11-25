# Run only on an isolated Windows CI runner. No GUI launch or desktop automation.
param([string]$Installer='dist/savesync-setup-dev.exe')
$ErrorActionPreference='Stop'
if (!$env:CI -or !$env:RUNNER_TEMP) { throw 'Installer smoke is restricted to CI' }
$root=Join-Path $env:RUNNER_TEMP ('savesync-installer-smoke-'+[Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $root | Out-Null
$installation=Join-Path $root 'app'
$env:APPDATA=Join-Path $root 'profile'
$env:LOCALAPPDATA=Join-Path $root 'local'
$env:SAVESYNC_SOCKET='installer-smoke-'+[Guid]::NewGuid().ToString('N')
$env:SAVESYNC_STEAM_ROOT=Join-Path $root 'empty-steam'
$installerPath=(Resolve-Path $Installer).Path
$daemon=$null
function Install-Package {
    $arguments=@('/VERYSILENT','/SUPPRESSMSGBOXES','/NORESTART',('/DIR="'+$installation+'"'),'/TASKS=autostart',('/LOG="'+(Join-Path $root 'install.log')+'"'))
    $process=Start-Process -FilePath $installerPath -ArgumentList $arguments -WindowStyle Hidden -Wait -PassThru
    if ($process.ExitCode -ne 0) { throw "Installer failed: $($process.ExitCode)" }
}
try {
    Install-Package
    foreach ($name in @('savesync-app.exe','savesync-daemon.exe','unins000.exe')) { if (!(Test-Path -LiteralPath (Join-Path $installation $name))) { throw "Missing installed $name" } }
    $run=(Get-ItemProperty -LiteralPath 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run').SaveSync
    if ($run -notlike ('*'+$installation+'*--tray')) { throw 'User autostart was not registered' }
    $executable=Join-Path $installation 'savesync-daemon.exe'
    $daemon=Start-Process -FilePath $executable -ArgumentList 'serve' -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $root 'daemon.out') -RedirectStandardError (Join-Path $root 'daemon.err')
    $ready=$false
    for ($attempt=0;$attempt -lt 60;$attempt++) { $null=& $executable status; if ($LASTEXITCODE -eq 0) {$ready=$true;break}; Start-Sleep -Milliseconds 100 }
    if (!$ready) { throw 'Installed daemon did not start' }
    $configPath=Join-Path $env:APPDATA 'savesync/config.json'
    $identity=(Get-Content -LiteralPath $configPath -Raw|ConvertFrom-Json).device_id
    $marker=Join-Path $env:APPDATA 'savesync/keep-user-data.txt'
    Set-Content -LiteralPath $marker -Value 'preserve this'
    # Upgrade must ask the running daemon to stop instead of killing a transaction.
    Install-Package
    if (!$daemon.WaitForExit(10000) -or $daemon.ExitCode -ne 0) { throw 'Upgrade did not stop the daemon gracefully' }
    if ((Get-Content -LiteralPath $configPath -Raw|ConvertFrom-Json).device_id -ne $identity) { throw 'Upgrade changed device identity' }
    $uninstaller=Join-Path $installation 'unins000.exe'
    $process=Start-Process -FilePath $uninstaller -ArgumentList '/VERYSILENT','/SUPPRESSMSGBOXES','/NORESTART' -WindowStyle Hidden -Wait -PassThru
    if ($process.ExitCode -ne 0) { throw 'Uninstall failed' }
    if (Test-Path -LiteralPath $executable) { throw 'Uninstall left executable behind' }
    if (!(Test-Path -LiteralPath $marker)) { throw 'Uninstall deleted user data' }
    $run=Get-ItemProperty -LiteralPath 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' -Name SaveSync -ErrorAction SilentlyContinue
    if ($run) { throw 'Uninstall left autostart behind' }
    Write-Output 'PASS: silent per-user install, HKCU autostart, graceful upgrade, uninstall, preserved profile'
} finally {
    if ($daemon -and !$daemon.HasExited) { Stop-Process -Id $daemon.Id -Force }
    $target=[IO.Path]::GetFullPath($root)
    $workspace=[IO.Path]::GetFullPath($env:RUNNER_TEMP).TrimEnd('\')+'\'
    if (!$target.StartsWith($workspace,[StringComparison]::OrdinalIgnoreCase)) { throw 'Unsafe CI cleanup path' }
    Remove-Item -LiteralPath $target -Recurse -Force
}
