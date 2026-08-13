$ErrorActionPreference = "Stop"
$releaseBaseUrl = if ($env:MOLI_RELEASE_BASE_URL) {
    $env:MOLI_RELEASE_BASE_URL.TrimEnd("/")
} else {
    "https://github.com/lexmount/moli/releases/latest/download"
}
$assetName = "moli-x86_64-pc-windows-msvc.zip"
$installDir = if ($env:MOLI_INSTALL_DIR) {
    $env:MOLI_INSTALL_DIR
} else {
    Join-Path $env:LOCALAPPDATA "Moli\bin"
}
$tempDir = Join-Path ([System.IO.Path]::GetTempPath()) (
    "moli-install-" + [System.Guid]::NewGuid().ToString("N")
)

New-Item -ItemType Directory -Path $tempDir | Out-Null
try {
    $archivePath = Join-Path $tempDir $assetName
    $extractDir = Join-Path $tempDir "package"

    Write-Host "Downloading $assetName..."
    Invoke-WebRequest -UseBasicParsing -Uri "$releaseBaseUrl/$assetName" -OutFile $archivePath

    Expand-Archive -LiteralPath $archivePath -DestinationPath $extractDir
    $packageDirs = @(
        Get-ChildItem -LiteralPath $extractDir -Directory |
            Where-Object { Test-Path -LiteralPath (Join-Path $_.FullName "moli.exe") }
    )
    if ($packageDirs.Count -ne 1) {
        throw "Downloaded archive does not contain a single Moli package."
    }
    $packageDir = $packageDirs[0].FullName

    New-Item -ItemType Directory -Force -Path $installDir | Out-Null
    Copy-Item -LiteralPath (Join-Path $packageDir "moli.exe") `
        -Destination (Join-Path $installDir "moli.exe") -Force
    Write-Host "Installed moli to $(Join-Path $installDir 'moli.exe')"

    $pathEntries = $env:PATH -split ";"
    if ($installDir -notin $pathEntries) {
        Write-Host "Add $installDir to PATH, then run: moli version"
    }
} finally {
    if (Test-Path -LiteralPath $tempDir) {
        Remove-Item -LiteralPath $tempDir -Recurse -Force
    }
}
