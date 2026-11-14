# Optional: turns a Windows 11 x64 PC into a self-hosted GitHub Actions runner for this repository. Windows installers
# are already cross-built on the Linux runners; a real Windows runner adds a native build and tests that run on
# Windows itself. Run in an elevated PowerShell:
#
#   powershell -ExecutionPolicy Bypass -File tools\runners\windows-runner.ps1 -Repo webxlr8/openvideoeditor -Token <token>
#
# The registration token comes from the repository's Settings -> Actions -> Runners -> New self-hosted runner (valid
# for one hour). Installs Visual Studio 2022 Build Tools (C++), Git, Python, Node and pnpm, Chocolatey with NASM and
# ffmpeg (test media only), vcpkg with dav1d, then the official runner as a Windows service with the labels CI asks for
# (self-hosted, Windows, X64, plus ove-windows). Re-running it is safe.
param(
  [Parameter(Mandatory = $true)][string]$Repo,
  [Parameter(Mandatory = $true)][string]$Token,
  [string]$Name = "$env:COMPUTERNAME-ove",
  [string]$Dir = "C:\actions-runner",
  [string]$RunnerVersion = "2.338.2"
)
$ErrorActionPreference = "Stop"
function Step($t) { Write-Host "`n== $t" }

if (-not ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole(
    [Security.Principal.WindowsBuiltInRole]::Administrator)) { throw "Run this in an elevated PowerShell." }

Step "winget packages"
$pkgs = @("Git.Git", "Python.Python.3.12", "OpenJS.NodeJS.LTS", "Microsoft.VisualStudio.2022.BuildTools")
foreach ($p in $pkgs) {
  if ($p -eq "Microsoft.VisualStudio.2022.BuildTools") {
    winget install --id $p -e --accept-source-agreements --accept-package-agreements `
      --override "--quiet --wait --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended" | Out-Null
  } else {
    winget install --id $p -e --accept-source-agreements --accept-package-agreements | Out-Null
  }
}
$env:Path = [Environment]::GetEnvironmentVariable("Path", "Machine") + ";" + [Environment]::GetEnvironmentVariable("Path", "User")
npm install -g pnpm@10 | Out-Null

Step "Chocolatey, NASM, ffmpeg (test media only)"
if (-not (Get-Command choco -ErrorAction SilentlyContinue)) {
  Set-ExecutionPolicy Bypass -Scope Process -Force
  [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor 3072
  Invoke-Expression ((New-Object Net.WebClient).DownloadString("https://community.chocolatey.org/install.ps1"))
}
choco install -y nasm ffmpeg | Out-Null

Step "vcpkg with dav1d"
if (-not (Test-Path "C:\vcpkg\vcpkg.exe")) {
  git clone https://github.com/microsoft/vcpkg C:\vcpkg
  & C:\vcpkg\bootstrap-vcpkg.bat -disableMetrics
}
& C:\vcpkg\vcpkg.exe install dav1d:x64-windows
[Environment]::SetEnvironmentVariable("VCPKG_ROOT", "C:\vcpkg", "Machine")
$machinePath = [Environment]::GetEnvironmentVariable("Path", "Machine")
if ($machinePath -notlike "*C:\vcpkg*") { [Environment]::SetEnvironmentVariable("Path", "$machinePath;C:\vcpkg", "Machine") }

Step "GitHub Actions runner $RunnerVersion in $Dir"
New-Item -ItemType Directory -Force -Path $Dir | Out-Null
Set-Location $Dir
if (-not (Test-Path ".\config.cmd")) {
  $zip = "actions-runner-win-x64-$RunnerVersion.zip"
  Invoke-WebRequest -Uri "https://github.com/actions/runner/releases/download/v$RunnerVersion/$zip" -OutFile $zip
  Expand-Archive -Path $zip -DestinationPath . -Force
  Remove-Item $zip
}

Step "Register with github.com/$Repo as a service"
if (-not (Test-Path ".\.runner")) {
  .\config.cmd --unattended --url "https://github.com/$Repo" --token $Token --name $Name `
    --labels "ove-windows" --work _work --runasservice --replace
} else { Write-Host "Already registered." }

Write-Host "`nDone. The runner '$Name' should show as Idle under Settings -> Actions -> Runners."
Write-Host "It answers jobs labelled self-hosted, Windows, X64 (CI's default) and ove-windows."
