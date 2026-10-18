# Windows: build the installer around an omacus.exe and check it the way people use
# it. CI runs this on the release build (release.yml) and on every change (ci.yml).
#
#   pwsh tests/windows-installer.ps1 target\release\omacus.exe
#
# 1. omacus.exe runs on a fresh Windows: it doesn't need the Visual C++ runtime
#    (VCRUNTIME140.dll isn't there until something installs it; GitHub's runners
#    have it, so running the .exe here wouldn't catch that).
# 2. Builds target\installer\omacus-windows-x86_64-setup.exe with Inno Setup.
# 3. Installs it with the one-liner (install.ps1, in Windows PowerShell 5.1, as
#    `irm | iex` runs it) from a local server: Start menu entry, PATH, the right
#    version. Then again over the top, as an update. The window opens. Then
#    uninstalls it.

param([Parameter(Mandatory)][string]$Exe)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root
$Exe = (Resolve-Path $Exe).Path
$version = (Select-String -Path Cargo.toml -Pattern '^version = "(.*)"' | Select-Object -First 1).Matches[0].Groups[1].Value
function Check($ok, $what) {
  if (-not $ok) { throw "FAILED: $what" }
  Write-Host "ok   $what"
}

# 1. No C runtime DLLs: their names would be in the import table.
$bytes = [IO.File]::ReadAllBytes($Exe)
$ascii = [Text.Encoding]::ASCII.GetString($bytes)
foreach ($dll in 'VCRUNTIME140', 'MSVCP140', 'api-ms-win-crt-') {
  Check (-not $ascii.Contains($dll)) "omacus.exe doesn't need $dll*.dll"
}
# The icon and version details are compiled in together (build.rs).
Check ((Get-Item $Exe).VersionInfo.FileDescription -eq 'Omacus') 'omacus.exe carries its icon and details'

# 2. The installer.
$iscc = "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe"
if (-not (Test-Path $iscc)) {
  choco install innosetup --no-progress -y | Out-Null
}
Check (Test-Path $iscc) 'Inno Setup is installed'
& $iscc /Q "/DVersion=$version" "/DExe=$Exe" installer\omacus.iss
Check ($LASTEXITCODE -eq 0) 'the installer builds'
$setup = Join-Path $root 'target\installer\omacus-windows-x86_64-setup.exe'
Check (Test-Path $setup) "built $setup"

# 3. The one-liner, against a local copy of the release files.
$site = Join-Path $env:RUNNER_TEMP 'omacus-release'
New-Item -ItemType Directory -Force -Path $site | Out-Null
Copy-Item $setup $site
$hash = (Get-FileHash -Algorithm SHA256 $setup).Hash.ToLower()
"$hash  omacus-windows-x86_64-setup.exe" | Set-Content -Encoding ascii (Join-Path $site 'SHA256SUMS.txt')
$server = Start-Process -PassThru -WindowStyle Hidden python -ArgumentList '-m', 'http.server', '8765', '--bind', '127.0.0.1', '--directory', $site
Start-Sleep 2
$env:OMACUS_RELEASE_URL = 'http://127.0.0.1:8765'
$env:OMACUS_NO_LAUNCH = '1'
$app = Join-Path $env:LOCALAPPDATA 'Programs\Omacus'
$installed = Join-Path $app 'omacus.exe'
$shortcut = Join-Path ([Environment]::GetFolderPath('Programs')) 'Omacus.lnk'
try {
  foreach ($round in 'install', 'update') {
    powershell -NoProfile -ExecutionPolicy Bypass -Command "Get-Content -Raw '$root\install.ps1' | Invoke-Expression"
    Check (Test-Path $installed) "$round`: omacus.exe is in $app"
    $out = (& $installed --version | Out-String).Trim()
    Check ($out -eq "omacus $version") "$round`: it runs ($out)"
    Check (Test-Path $shortcut) "$round`: Omacus is in the Start menu"
    $path = [Environment]::GetEnvironmentVariable('Path', 'User')
    $entries = @($path -split ';' | Where-Object { $_ -eq $app })
    Check ($entries.Count -eq 1) "$round`: the install folder is on PATH once"
  }
  $answer = (& $installed '2 hours + 30 min in minutes' | Out-String).Trim()
  Check ($answer -eq '150 minutes') "the installed app answers ($answer)"

  # The window opens, on a machine with no GPU (GitHub's runners have none, like
  # many virtual machines): OMACUS_SCREENSHOT saves it, then closes it. With
  # OpenGL it used to quit at once, "egui_glow requires opengl 2.0+".
  $shot = Join-Path $env:RUNNER_TEMP 'omacus-window.png'
  $err = Join-Path $env:RUNNER_TEMP 'omacus-window.err'
  $env:OMACUS_SCREENSHOT = $shot
  $env:OMACUS_NO_UPDATE_CHECK = '1'
  $p = Start-Process -PassThru -FilePath $installed -RedirectStandardError $err
  $done = $p.WaitForExit(60000)
  if (-not $done) { Stop-Process -Id $p.Id -ErrorAction SilentlyContinue }
  Remove-Item Env:OMACUS_SCREENSHOT, Env:OMACUS_NO_UPDATE_CHECK
  $said = (Get-Content -Raw $err -ErrorAction SilentlyContinue) -as [string]
  Check ($done -and $p.ExitCode -eq 0 -and (Test-Path $shot)) "the window opens and draws ($($said.Trim()))"

  # A tampered download is refused.
  [IO.File]::WriteAllBytes((Join-Path $site 'omacus-windows-x86_64-setup.exe'), [byte[]](1, 2, 3))
  $said = powershell -NoProfile -ExecutionPolicy Bypass -Command "Get-Content -Raw '$root\install.ps1' | Invoke-Expression" *>&1 | Out-String
  Check ($said -match "doesn't match its checksum") 'a tampered download is refused'
  Check ((& $installed --version | Out-String).Trim() -eq "omacus $version") 'and the installed app is untouched'

  # Uninstall: gone, and off the PATH.
  $p = Start-Process -Wait -PassThru (Join-Path $app 'unins000.exe') -ArgumentList '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART'
  Check ($p.ExitCode -eq 0) 'the uninstaller runs'
  # It finishes from a copy of itself in TEMP, after this one has returned.
  for ($i = 0; $i -lt 60 -and (Test-Path $installed); $i++) { Start-Sleep -Milliseconds 500 }
  Check (-not (Test-Path $installed)) 'uninstalled: omacus.exe is gone'
  Check (-not (Test-Path $shortcut)) 'uninstalled: the Start menu entry is gone'
  $path = [Environment]::GetEnvironmentVariable('Path', 'User')
  Check (-not (@($path -split ';') -contains $app)) 'uninstalled: off the PATH'
} finally {
  Stop-Process -Id $server.Id -ErrorAction SilentlyContinue
}
