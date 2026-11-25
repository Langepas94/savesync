param([Parameter(Mandatory=$true)][string]$Path,[Parameter(Mandatory=$true)][string]$ExpectedSigner,[Parameter(Mandatory=$true)][string]$ExpectedVersion)
$ErrorActionPreference = 'Stop'
try {
    $signature = Get-AuthenticodeSignature -LiteralPath $Path
    if ($signature.Status -ne 'Valid' -or !$signature.SignerCertificate) { throw 'Invalid Authenticode signature' }
    $algorithm = [System.Security.Cryptography.SHA256]::Create()
    try { $fingerprint = ([BitConverter]::ToString($algorithm.ComputeHash($signature.SignerCertificate.RawData))).Replace('-', '') }
    finally { $algorithm.Dispose() }
    if ($fingerprint -ine $ExpectedSigner) { throw 'Unexpected publisher certificate' }
    $actual = [Version](Get-Item -LiteralPath $Path).VersionInfo.ProductVersion
    $expected = [Version]$ExpectedVersion
    if ($actual.Major -ne $expected.Major -or $actual.Minor -ne $expected.Minor -or $actual.Build -ne $expected.Build) { throw 'Unexpected installer version' }
    exit 0
} catch { exit 1 }
