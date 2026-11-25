param([Parameter(Mandatory=$true)][string]$Path)
$ErrorActionPreference = 'Stop'
if (!$env:SAVESYNC_SIGNING_THUMBPRINT) { throw 'Signing certificate is not configured' }
$tools = Get-ChildItem -LiteralPath "${env:ProgramFiles(x86)}\Windows Kits\10\bin" -Filter signtool.exe -Recurse | Where-Object { $_.FullName -match '\\x64\\' } | Sort-Object FullName -Descending
$tool = $tools | Select-Object -First 1
if (!$tool) { throw 'Windows SDK signtool missing' }
$timestamp = if ($env:SAVESYNC_TIMESTAMP_URL) { $env:SAVESYNC_TIMESTAMP_URL } else { 'https://timestamp.digicert.com' }
& $tool.FullName sign /sha1 $env:SAVESYNC_SIGNING_THUMBPRINT /fd SHA256 /tr $timestamp /td SHA256 $Path
if ($LASTEXITCODE -ne 0) { throw 'Authenticode signing failed or returned warnings' }
& $tool.FullName verify /pa /all /tw $Path
if ($LASTEXITCODE -ne 0) { throw 'Authenticode verification failed or returned warnings' }
