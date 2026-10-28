# 全量验证：workspace 测试 + 新 crate clippy + codegen 漂移 + 头文件一致性
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Push-Location $root
$failed = @()

function Step($name, $block) {
    Write-Host ""
    Write-Host "=== $name ===" -ForegroundColor Cyan
    & $block
    if ($LASTEXITCODE -ne 0) { $script:failed += $name; Write-Host "FAILED: $name" -ForegroundColor Red }
}

Step "cargo test --workspace"   { cargo test --workspace }
Step "clippy (new crates)"      { cargo clippy -p slive-core -p slive-host -p slive-ffi --all-targets -- -D warnings }
Step "clippy (bridge)"          { cargo clippy -p rust_lib_simple_live_app --all-targets }
Step "header sync"              { cargo test -p slive-ffi --test header_sync }
Step "codegen drift"            { git diff --exit-code src/frb_generated.rs lib/src/rust/ }

Pop-Location
Write-Host ""
if ($failed.Count -gt 0) {
    Write-Host "失败：$($failed -join ', ')" -ForegroundColor Red
    exit 1
}
Write-Host "全部通过" -ForegroundColor Green
