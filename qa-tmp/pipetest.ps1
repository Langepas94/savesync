$ErrorActionPreference = 'Stop'
try {
    $pipe = New-Object System.IO.Pipes.NamedPipeClientStream('.', 'savesync-daemon', [System.IO.Pipes.PipeDirection]::InOut)
    $pipe.Connect(2000)
    $req = '{"id":1,"method":"status"}' + "`n"
    $bytes = [System.Text.Encoding]::UTF8.GetBytes($req)
    $pipe.Write($bytes, 0, $bytes.Length)
    $pipe.Flush()
    $ms = New-Object System.IO.MemoryStream
    $buf = New-Object byte[] 65536
    do {
        $n = $pipe.Read($buf, 0, $buf.Length)
        $ms.Write($buf, 0, $n)
    } while (-not ([System.Text.Encoding]::UTF8.GetString($ms.ToArray()).Contains("`n")))
    Write-Output ([System.Text.Encoding]::UTF8.GetString($ms.ToArray()).Trim())
    $pipe.Close()
} catch {
    Write-Output ("FAIL: " + $_.Exception.Message)
}
