# 构建 Simple Live C ABI（Windows）
#
# 默认产出 cdylib（slive_ffi.dll）+ staticlib（slive_ffi.lib）。
# 加 -StaticCrt 可启用静态 CRT（宿主必须同口径，否则 MSVC 报 LNK2038）。
param(
    [switch]$StaticCrt,
    [switch]$Release
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Push-Location $root
try {
    $args = @("build", "-p", "slive-ffi", "--target", "x86_64-pc-windows-msvc")
    if ($Release) { $args += "--release" }
    if ($StaticCrt) { $env:RUSTFLAGS = "-C target-feature=+crt-static" }
    & cargo @args
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

    $profile = if ($Release) { "release" } else { "debug" }
    $outDir = Join-Path $root "target/x86_64-pc-windows-msvc/$profile"
    Write-Host ""
    Write-Host "产物："
    Write-Host "  $outDir/slive_ffi.dll"
    Write-Host "  $outDir/slive_ffi.lib"
    Write-Host "头文件：$PSScriptRoot/include/slive.h（C）/ slive.hpp（C++）"
} finally {
    Pop-Location
}
