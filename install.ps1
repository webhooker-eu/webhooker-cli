# Installs whk, the Webhooker CLI, on Windows.
#
#   irm https://webhooker.eu/install.ps1 | iex
#
# Set $env:WHK_VERSION (for example v0.1.1) or $env:WHK_INSTALL_DIR before running
# to pin a release or change where whk goes.

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

function Install-Whk {
    $repository = 'webhooker-eu/webhooker-cli'
    $archiveName = 'whk-x86_64-pc-windows-msvc.zip'
    $checksumName = 'whk-x86_64-pc-windows-msvc.sha256'
    $requestedVersion = if ($env:WHK_VERSION) { $env:WHK_VERSION } else { 'latest' }
    $installDirectory = if ($env:WHK_INSTALL_DIR) { $env:WHK_INSTALL_DIR } else { Join-Path $env:LOCALAPPDATA 'Programs\whk' }

    # Windows PowerShell 5.1 may default to TLS 1.0, which GitHub refuses.
    [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12

    if ($requestedVersion -eq 'latest') {
        $releaseUrl = "https://github.com/$repository/releases/latest/download"
    } else {
        if (-not $requestedVersion.StartsWith('v')) { $requestedVersion = "v$requestedVersion" }
        $releaseUrl = "https://github.com/$repository/releases/download/$requestedVersion"
    }

    $temporaryDirectory = Join-Path ([IO.Path]::GetTempPath()) ([Guid]::NewGuid().ToString())
    New-Item -ItemType Directory -Path $temporaryDirectory | Out-Null

    try {
        $archivePath = Join-Path $temporaryDirectory $archiveName
        $checksumPath = Join-Path $temporaryDirectory $checksumName

        Write-Host "Downloading $archiveName ($requestedVersion)"
        Invoke-WebRequest -UseBasicParsing -Uri "$releaseUrl/$archiveName" -OutFile $archivePath
        Invoke-WebRequest -UseBasicParsing -Uri "$releaseUrl/$checksumName" -OutFile $checksumPath

        $expectedChecksum = ((Get-Content -Raw $checksumPath).Trim() -split '\s+')[0].ToLowerInvariant()
        $actualChecksum = (Get-FileHash -Algorithm SHA256 $archivePath).Hash.ToLowerInvariant()
        if ($expectedChecksum -ne $actualChecksum) {
            throw "Checksum mismatch for $archiveName (expected $expectedChecksum, got $actualChecksum)"
        }

        Expand-Archive -Path $archivePath -DestinationPath $temporaryDirectory -Force
        New-Item -ItemType Directory -Path $installDirectory -Force | Out-Null
        Move-Item -Force (Join-Path $temporaryDirectory 'whk.exe') (Join-Path $installDirectory 'whk.exe')
    } finally {
        Remove-Item -Recurse -Force $temporaryDirectory -ErrorAction SilentlyContinue
    }

    $installedVersion = & (Join-Path $installDirectory 'whk.exe') --version
    Write-Host "Installed $installedVersion to $installDirectory"

    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    $userPathEntries = if ($userPath) { $userPath -split ';' } else { @() }
    if ($userPathEntries -notcontains $installDirectory) {
        $updatedUserPath = (@($userPathEntries | Where-Object { $_ }) + $installDirectory) -join ';'
        [Environment]::SetEnvironmentVariable('Path', $updatedUserPath, 'User')
        $env:Path = "$env:Path;$installDirectory"
        Write-Host "Added $installDirectory to your user PATH. Open a new terminal to use whk."
    }

    Write-Host ''
    Write-Host 'Next: create an API key at https://app.webhooker.eu and run: whk login'
}

Install-Whk
