<#
.SYNOPSIS
  Downloads the official llama.cpp server (the built-in LLM engine) so you can
  test the "Built-in (Local)" assistant provider locally.

.DESCRIPTION
  SpeakoFlow's built-in LLM runs a small `llama-server` engine in the
  background. In production this binary is bundled in the installer; for local
  development you fetch it once with this script.

  The script downloads the llama.cpp Windows build the app is pinned to
  (PINNED_ENGINE_TAG in src-tauri/src/managers/local_llm.rs), extracts it, and
  prints the exact command to launch the app pointing at it. It never asks
  GitHub for the "latest" release: llama.cpp marks every real build as a
  prerelease, so "latest" is a stub with no binaries in it.

.PARAMETER Backend
  Which build to download: "vulkan" (default, GPU on most machines),
  "cpu" (works everywhere, slower), or "cuda" (NVIDIA only).

.PARAMETER Tag
  The llama.cpp build to download (e.g. b11429). Defaults to the tag the app
  is pinned to.

.PARAMETER CudaVersion
  With -Backend cuda, the CUDA version of the build (e.g. 12.4 or 13.4).
  Defaults to the highest one the release has. Your NVIDIA driver must support
  it.

.PARAMETER DryRun
  Print the archives that would be downloaded, and stop.

.EXAMPLE
  ./scripts/setup-llm-engine.ps1
  ./scripts/setup-llm-engine.ps1 -Backend cpu
  ./scripts/setup-llm-engine.ps1 -Backend cuda -CudaVersion 12.4
  ./scripts/setup-llm-engine.ps1 -Tag b11531 -DryRun
#>
param(
    [ValidateSet("vulkan", "cpu", "cuda")]
    [string]$Backend = "vulkan",
    [string]$Tag,
    [string]$CudaVersion,
    [switch]$DryRun
)

$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $PSScriptRoot
$engineDir = Join-Path $repoRoot "src-tauri\resources\engine"

if (-not $Tag) {
    $source = Join-Path $repoRoot "src-tauri\src\managers\local_llm.rs"
    $match = Select-String -Path $source -Pattern 'const PINNED_ENGINE_TAG: &str = "([^"]+)";' |
        Select-Object -First 1
    if (-not $match) {
        Write-Host "Could not find PINNED_ENGINE_TAG in $source. Pass -Tag." -ForegroundColor Red
        exit 1
    }
    $Tag = $match.Matches[0].Groups[1].Value
}

Write-Host "Fetching the asset list of llama.cpp $Tag..."
try {
    $release = Invoke-RestMethod `
        -Uri "https://api.github.com/repos/ggml-org/llama.cpp/releases/tags/$Tag" `
        -Headers @{ "User-Agent" = "speakoflow-setup" } `
        -TimeoutSec 60
} catch {
    Write-Host "Could not read llama.cpp release $Tag ($($_.Exception.Message))." -ForegroundColor Red
    exit 1
}

$names = @($release.assets | ForEach-Object { $_.name })

function Find-Asset([string]$name) {
    $release.assets | Where-Object { $_.name -eq $name } | Select-Object -First 1
}

$assets = @()
if ($Backend -eq "cuda") {
    # The CUDA archive carries llama.cpp's own ggml-cuda.dll but not the CUDA
    # runtime it links against (cudart, cublas), which ships as a separate
    # archive per CUDA version and is needed unless the CUDA toolkit is on PATH.
    $prefix = "llama-$Tag-bin-win-cuda-"
    $versions = @($names |
        Where-Object { $_.StartsWith($prefix) -and $_.EndsWith("-x64.zip") } |
        ForEach-Object { $_.Substring($prefix.Length, $_.Length - $prefix.Length - "-x64.zip".Length) } |
        Sort-Object { [version]$_ } -Descending)
    if ($versions.Count -eq 0) {
        Write-Host "Release $Tag has no CUDA build for Windows x64." -ForegroundColor Yellow
        exit 1
    }
    if ($CudaVersion) {
        if ($versions -notcontains $CudaVersion) {
            Write-Host "Release $Tag has no CUDA $CudaVersion build. It has: $($versions -join ', ')" -ForegroundColor Yellow
            exit 1
        }
        $cuda = $CudaVersion
    } else {
        $cuda = $versions[0]
    }
    $wanted = @("llama-$Tag-bin-win-cuda-$cuda-x64.zip", "cudart-llama-bin-win-cuda-$cuda-x64.zip")
} else {
    $wanted = @("llama-$Tag-bin-win-$Backend-x64.zip")
}

foreach ($name in $wanted) {
    $asset = Find-Asset $name
    if (-not $asset) {
        Write-Host "Release $Tag has no asset named $name." -ForegroundColor Yellow
        Write-Host "Windows assets in that release:"
        $names | Where-Object { $_ -like "*win*" } | ForEach-Object { Write-Host "  $_" }
        exit 1
    }
    $assets += $asset
}

Write-Host "Resolved llama.cpp $Tag ($Backend):"
foreach ($asset in $assets) {
    Write-Host "  $($asset.name) ($([math]::Round($asset.size / 1MB)) MB)"
}
if ($DryRun) {
    Write-Host "Dry run: nothing downloaded."
    exit 0
}

New-Item -ItemType Directory -Force -Path $engineDir | Out-Null
foreach ($asset in $assets) {
    $zipPath = Join-Path $env:TEMP $asset.name
    Write-Host "Downloading $($asset.name)..."
    Invoke-WebRequest -Uri $asset.browser_download_url -OutFile $zipPath

    Write-Host "Extracting to $engineDir ..."
    Expand-Archive -Path $zipPath -DestinationPath $engineDir -Force
    Remove-Item $zipPath -Force
}

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
