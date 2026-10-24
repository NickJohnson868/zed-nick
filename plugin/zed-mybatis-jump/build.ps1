$ErrorActionPreference = 'Stop'
$env:CARGO_HOME = 'C:\Users\62453\Documents\tools\rust\cargo'
$env:RUSTUP_HOME = 'C:\Users\62453\Documents\tools\rust\rustup'
$env:PATH = "$env:CARGO_HOME\bin;$env:PATH"

if (!$env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR = Join-Path $env:USERPROFILE 'Documents\tools\rust\targets\zed-mybatis-jump' }

Push-Location $PSScriptRoot
try {
    cargo fmt --check
    if ($LASTEXITCODE -ne 0) { throw '格式检查失败' }
    cargo test --locked -p mybatis-jump-lsp
    if ($LASTEXITCODE -ne 0) { throw '跳转规则测试失败' }
    cargo clippy --locked -p mybatis-jump-lsp -- -D warnings
    if ($LASTEXITCODE -ne 0) { throw '原生服务检查失败' }
    cargo build --locked --release -p mybatis-jump-lsp
    if ($LASTEXITCODE -ne 0) { throw '原生服务编译失败' }
    rustup target add wasm32-wasip2
    if ($LASTEXITCODE -ne 0) { throw '安装 WASM 目标失败' }
    cargo build --locked --release -p zed-mybatis-jump --target wasm32-wasip2
    if ($LASTEXITCODE -ne 0) { throw '扩展编译失败' }
    New-Item -ItemType Directory -Path bin -Force | Out-Null
    Copy-Item -LiteralPath (Join-Path $env:CARGO_TARGET_DIR 'release/mybatis-jump-lsp.exe') -Destination bin/mybatis-jump-lsp.exe -Force
    Copy-Item -LiteralPath (Join-Path $env:CARGO_TARGET_DIR 'wasm32-wasip2/release/zed_mybatis_jump.wasm') -Destination extension.wasm.tmp -Force
    Move-Item extension.wasm.tmp extension.wasm -Force
    Write-Host '构建完成：extension.wasm + bin/mybatis-jump-lsp.exe'
} finally {
    Pop-Location
}
