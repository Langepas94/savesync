param(
    [Parameter(Mandatory=$true)][string]$Request
)
$ErrorActionPreference = 'Stop'
$pipe = New-Object System.IO.Pipes.NamedPipeClientStream('.', 'savesync-daemon', [System.IO.Pipes.PipeDirection]::InOut)
$pipe.Connect(3000)
$req = $Request + "`n"
$bytes = [System.Text.Encoding]::UTF8.GetBytes($req)
$pipe.Write($bytes, 0, $bytes.Length)
$pipe.Flush()
$ms = New-Object System.IO.MemoryStream
$buf = New-Object byte[] 1048576
$guard = 0
do {
    $n = $pipe.Read($buf, 0, $buf.Length)
    if ($n -le 0) { break }
    $ms.Write($buf, 0, $n)
    $guard++
    if ($guard -gt 100) { break }
} while (-not ([System.Text.Encoding]::UTF8.GetString($ms.ToArray()).Contains("`n")))
$pipe.Close()
Write-Output ([System.Text.Encoding]::UTF8.GetString($ms.ToArray()).Trim())
