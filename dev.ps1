#Requires -Version 5
<#
  Handy Ultra dev launcher.

  Run from anywhere:   .\dev.ps1     (or right-click > Run with PowerShell)
  Starts the app in development mode: hot-reloads UI changes instantly,
  rebuilds Rust changes automatically. Press Ctrl+C in this window (or
  close it) to stop the app.
#>

# Always run from the repo root, regardless of where the script was invoked.
Set-Location -Path $PSScriptRoot

# Fresh PATH (bun/cargo may have been installed after this shell's PATH was baked).
$env:Path = [Environment]::GetEnvironmentVariable('Path', 'Machine') + ';' +
            [Environment]::GetEnvironmentVariable('Path', 'User')

# Build environment required on Windows (see BUILD.md).
$env:VULKAN_SDK = [Environment]::GetEnvironmentVariable('VULKAN_SDK', 'Machine')
if (-not $env:LIBCLANG_PATH) {
    $env:LIBCLANG_PATH = 'C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Tools\Llvm\x64\bin'
}

Write-Host "Starting Handy Ultra in dev mode (Ctrl+C to stop)..." -ForegroundColor Cyan
bun tauri dev
