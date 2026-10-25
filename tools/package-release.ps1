param(
    [string]$ExecutablePath
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$version = '0.1.6'
$dist = Join-Path $root 'dist'
$packageName = "ExMonitorViewer-v$version-windows-x64"
$staging = Join-Path $dist $packageName
$zipPath = Join-Path $dist "$packageName.zip"
$executableSource = if ([string]::IsNullOrWhiteSpace($ExecutablePath)) {
    Join-Path $dist 'exmonitor-viewer.exe'
} elseif ([System.IO.Path]::IsPathRooted($ExecutablePath)) {
    $ExecutablePath
} else {
    Join-Path $root $ExecutablePath
}
$distFullPath = [System.IO.Path]::GetFullPath($dist).TrimEnd([System.IO.Path]::DirectorySeparatorChar)
$stagingFullPath = [System.IO.Path]::GetFullPath($staging)

if (-not (Test-Path -LiteralPath $executableSource -PathType Leaf)) {
    throw "Release executable not found: $executableSource"
}
if ([System.IO.Path]::GetDirectoryName($stagingFullPath) -ne $distFullPath) {
    throw 'Refusing to replace a package directory outside dist.'
}

New-Item -ItemType Directory -Force -Path $dist | Out-Null
if (Test-Path -LiteralPath $stagingFullPath) {
    Remove-Item -LiteralPath $stagingFullPath -Recurse -Force
}
New-Item -ItemType Directory -Path $stagingFullPath | Out-Null
Copy-Item -LiteralPath $executableSource -Destination (Join-Path $stagingFullPath 'exmonitor-viewer.exe')
Copy-Item -LiteralPath (Join-Path $root 'README.md') -Destination $stagingFullPath
Copy-Item -LiteralPath (Join-Path $root 'LICENSE') -Destination $stagingFullPath
Copy-Item -LiteralPath (Join-Path $root 'docs\release-notes-v0.1.6.md') -Destination (Join-Path $stagingFullPath 'RELEASE-NOTES.md')

$checksums = Get-ChildItem -LiteralPath $stagingFullPath -File |
    Sort-Object Name |
    ForEach-Object {
        $hash = (Get-FileHash -Algorithm SHA256 -LiteralPath $_.FullName).Hash.ToLowerInvariant()
        "$hash  $($_.Name)"
    }
Set-Content -Encoding ascii -LiteralPath (Join-Path $stagingFullPath 'SHA256SUMS.txt') -Value $checksums

if (Test-Path -LiteralPath $zipPath) {
    Remove-Item -LiteralPath $zipPath -Force
}
Compress-Archive -Path (Join-Path $stagingFullPath '*') -DestinationPath $zipPath -CompressionLevel Optimal

$outerChecksums = @(
    @{ Path = (Join-Path $stagingFullPath 'exmonitor-viewer.exe'); Name = 'exmonitor-viewer.exe' },
    @{ Path = $zipPath; Name = "$packageName.zip" }
) | ForEach-Object {
    $hash = (Get-FileHash -Algorithm SHA256 -LiteralPath $_.Path).Hash.ToLowerInvariant()
    "$hash  $($_.Name)"
}
Set-Content -Encoding ascii -LiteralPath (Join-Path $dist 'SHA256SUMS.txt') -Value $outerChecksums
Write-Output "Created $zipPath"
