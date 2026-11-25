# Build dependency setup is restricted to disposable GitHub Windows runners.
$ErrorActionPreference = 'Stop'
$compiler = "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe"
if (Test-Path -LiteralPath $compiler) { exit 0 }
if (!$env:CI -or !$env:RUNNER_TEMP) { throw 'Install Inno Setup 6 manually outside CI' }
$path = Join-Path $env:RUNNER_TEMP 'innosetup-6.7.4.exe'
Invoke-WebRequest -Uri 'https://github.com/jrsoftware/issrc/releases/download/is-6_7_4/innosetup-6.7.4.exe' -OutFile $path
if ((Get-AuthenticodeSignature -LiteralPath $path).Status -ne 'Valid') { throw 'Inno Setup download signature is invalid' }
$process = Start-Process -FilePath $path -ArgumentList '/VERYSILENT','/SUPPRESSMSGBOXES','/NORESTART','/ALLUSERS' -WindowStyle Hidden -Wait -PassThru
if ($process.ExitCode -ne 0 -or !(Test-Path -LiteralPath $compiler)) { throw 'Inno Setup dependency installation failed' }
Remove-Item -LiteralPath $path -Force
