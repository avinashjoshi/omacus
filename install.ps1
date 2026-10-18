# Omacus for Windows, in one line (PowerShell):
#
#   irm https://omacus.com/install.ps1 | iex
#
# Downloads the latest installer from GitHub, checks it against the release's
# SHA256SUMS.txt, installs Omacus for you (no admin prompt) into
# %LOCALAPPDATA%\Programs\Omacus with a Start menu entry, and opens it.
# Running it again updates Omacus. Settings > Apps uninstalls it.
#
# For testing: $env:OMACUS_RELEASE_URL points at another folder of release files.

& {
  $ErrorActionPreference = 'Stop'
  $ProgressPreference = 'SilentlyContinue' # Windows PowerShell's progress bar makes downloads slow
  [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12

  $base = if ($env:OMACUS_RELEASE_URL) { $env:OMACUS_RELEASE_URL.TrimEnd('/') } else {
    'https://github.com/avinashjoshi/omacus/releases/latest/download'
  }
  $name = 'omacus-windows-x86_64-setup.exe'

  if (-not [Environment]::Is64BitOperatingSystem) {
    Write-Host 'Omacus needs 64-bit Windows.' -ForegroundColor Red
    return
  }

  $dir = Join-Path ([IO.Path]::GetTempPath()) ("omacus-" + [Guid]::NewGuid())
  New-Item -ItemType Directory -Path $dir | Out-Null
  try {
    Write-Host 'Downloading Omacus...'
    $setup = Join-Path $dir $name
    try {
      Invoke-WebRequest -UseBasicParsing -Uri "$base/$name" -OutFile $setup
    } catch {
      # Right after a release, its files take a few minutes to appear.
      throw "the installer couldn't be downloaded ($($_.Exception.Message)); if a release came out in the last few minutes, try again shortly"
    }
    $sums = (Invoke-WebRequest -UseBasicParsing -Uri "$base/SHA256SUMS.txt").Content
    if ($sums -is [byte[]]) { $sums = [Text.Encoding]::UTF8.GetString($sums) }

    $line = ($sums -split "`n") | Where-Object { $_ -match "\s\*?$([regex]::Escape($name))\s*$" } | Select-Object -First 1
    if (-not $line) { throw "SHA256SUMS.txt has no entry for $name" }
    $want = ($line -split '\s+')[0].ToLower()
    $got = (Get-FileHash -Algorithm SHA256 -Path $setup).Hash.ToLower()
    if ($got -ne $want) {
      throw "the download doesn't match its checksum (expected $want, got $got); nothing was installed"
    }

    Write-Host 'Installing...'
    $p = Start-Process -FilePath $setup -Wait -PassThru -ArgumentList '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/CURRENTUSER'
    if ($p.ExitCode -ne 0) { throw "the installer stopped with code $($p.ExitCode)" }

    $exe = Join-Path $env:LOCALAPPDATA 'Programs\Omacus\omacus.exe'
    if (-not (Test-Path $exe)) { throw "Omacus wasn't found at $exe after installing" }
    Write-Host "Omacus is installed. Find it in Start, or type 'omacus 2+2' in a new terminal." -ForegroundColor Green
    if (-not $env:OMACUS_NO_LAUNCH) { Start-Process -FilePath $exe }
  } catch {
    Write-Host "Omacus couldn't be installed: $($_.Exception.Message)" -ForegroundColor Red
    Write-Host 'Download the installer from https://omacus.com/download/ instead.'
  } finally {
    Remove-Item -Recurse -Force -Path $dir -ErrorAction SilentlyContinue
  }
}
