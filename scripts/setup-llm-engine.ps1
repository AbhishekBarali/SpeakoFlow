<#
.SYNOPSIS
  Downloads the official llama.cpp server (the built-in LLM engine) so you can
  test the "Built-in (Local)" assistant provider locally.

.DESCRIPTION
  SpeakoFlow's built-in LLM runs a small `llama-server` engine in the
  background. In production this binary is bundled in the installer; for local
  development you fetch it once with this script.

  The script downloads the latest llama.cpp Windows build from GitHub, extracts
  it, and prints the exact command to launch the app pointing at it.

.PARAMETER Backend
  Which build to download: "vulkan" (default, GPU on most machines),
  "cpu" (works everywhere, slower), or "cuda" (NVIDIA only).

.EXAMPLE
  ./scripts/setup-llm-engine.ps1
  ./scripts/setup-llm-engine.ps1 -Backend cpu
#>
param(
    [ValidateSet("vulkan", "cpu", "cuda")]
    [string]$Backend = "vulkan"
)

$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $PSScriptRoot
$engineDir = Join-Path $repoRoot "src-tauri\resources\engine"
New-Item -ItemType Directory -Force -Path $engineDir | Out-Null

Write-Host "Fetching latest llama.cpp release info..."
$release = Invoke-RestMethod `
    -Uri "https://api.github.com/repos/ggml-org/llama.cpp/releases/latest" `
    -Headers @{ "User-Agent" = "speakoflow-setup" }

$pattern = "win-$Backend-x64.zip"
$asset = $release.assets | Where-Object { $_.name -like "*$pattern" } | Select-Object -First 1

if (-not $asset) {
    Write-Host "Could not find a '$Backend' Windows build in release $($release.tag_name)." -ForegroundColor Yellow
    Write-Host "Available assets:"
    $release.assets | ForEach-Object { Write-Host "  $($_.name)" }
    exit 1
}

$zipPath = Join-Path $env:TEMP $asset.name
Write-Host "Downloading $($asset.name) ($([math]::Round($asset.size / 1MB)) MB)..."
Invoke-WebRequest -Uri $asset.browser_download_url -OutFile $zipPath

Write-Host "Extracting to $engineDir ..."
Expand-Archive -Path $zipPath -DestinationPath $engineDir -Force
Remove-Item $zipPath -Force

$exe = Get-ChildItem -Path $engineDir -Recurse -Filter "llama-server.exe" | Select-Object -First 1
if (-not $exe) {
    Write-Host "llama-server.exe not found after extraction." -ForegroundColor Red
    exit 1
}

Write-Host ""
Write-Host "Engine ready:" -ForegroundColor Green
Write-Host "  $($exe.FullName)"
Write-Host ""
Write-Host "Now launch the app pointing at it (same terminal):" -ForegroundColor Cyan
Write-Host "  `$env:HANDY_LLAMA_SERVER = `"$($exe.FullName)`""
Write-Host "  bun run tauri dev"
Write-Host ""
Write-Host "Then: Models tab -> Language Model -> download a model ->"
Write-Host "Assistant tab -> Provider: Built-in (Local) -> pick the model -> chat."
