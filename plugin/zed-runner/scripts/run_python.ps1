param(
    [Parameter(Mandatory=$true)][string]$Project,
    [Parameter(Mandatory=$true)][string]$Python,
    [string]$Uv,
    [string]$BasePython,
    [string]$Script
)
$ErrorActionPreference = 'Stop'
$Project = [IO.Path]::GetFullPath($Project)
$Python = [IO.Path]::GetFullPath($Python)
$venv = [IO.Path]::GetDirectoryName([IO.Path]::GetDirectoryName($Python))
$pyproject = Join-Path $Project 'pyproject.toml'
$requirements = Join-Path $Project 'requirements.txt'
$lock = Join-Path $Project 'uv.lock'
$manifests = @($pyproject, $requirements, $lock) | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf }
if (!$manifests.Count) { throw "未找到 Python 依赖文件：$Project" }
$fingerprint = ($manifests | ForEach-Object { "$_`t$((Get-FileHash -LiteralPath $_ -Algorithm SHA256).Hash)" }) -join "`n"
$stamp = Join-Path $venv '.zed-runner-deps.txt'
$ready = (Test-Path -LiteralPath $Python -PathType Leaf) -and
    (Test-Path -LiteralPath $stamp -PathType Leaf) -and
    ((Get-Content -LiteralPath $stamp -Raw) -eq $fingerprint)
if (!$ready) {
    if (Test-Path -LiteralPath $pyproject -PathType Leaf) {
        if (!$Uv -or ![IO.Path]::IsPathFullyQualified($Uv) -or !(Test-Path -LiteralPath $Uv -PathType Leaf)) {
            throw 'pyproject.toml 需要配置现有 uv.exe 的绝对路径。'
        }
        $previousEnvironment = $env:UV_PROJECT_ENVIRONMENT
        try {
            $env:UV_PROJECT_ENVIRONMENT = $venv
            $syncArguments = @('sync','--project',$Project,'--no-default-groups','--inexact')
            if ($BasePython) { $syncArguments += @('--python',$BasePython) }
            & $Uv @syncArguments
            if ($LASTEXITCODE -ne 0) { throw "uv sync 失败：$Project" }
        } finally {
            $env:UV_PROJECT_ENVIRONMENT = $previousEnvironment
        }
    }
    if (Test-Path -LiteralPath $requirements -PathType Leaf) {
        if (!$BasePython -or ![IO.Path]::IsPathFullyQualified($BasePython) -or !(Test-Path -LiteralPath $BasePython -PathType Leaf)) {
            throw 'requirements.txt 需要配置现有 python.exe 的绝对路径。'
        }
        if (!(Test-Path -LiteralPath $Python -PathType Leaf)) {
            & $BasePython -m venv $venv
            if ($LASTEXITCODE -ne 0) { throw "Python 创建虚拟环境失败：$venv" }
        }
        if ($Uv) {
            if (![IO.Path]::IsPathFullyQualified($Uv) -or !(Test-Path -LiteralPath $Uv -PathType Leaf)) {
                throw 'python.uv 必须是现有 uv.exe 的绝对路径。'
            }
            & $Uv pip install --python $Python -r $requirements
            if ($LASTEXITCODE -ne 0) { throw "uv 安装 requirements.txt 失败：$requirements" }
        } else {
            & $Python -m ensurepip --upgrade
            if ($LASTEXITCODE -ne 0) { throw "虚拟环境无法准备 pip：$Python" }
            & $Python -m pip install -r $requirements
            if ($LASTEXITCODE -ne 0) { throw "pip 安装依赖失败：$requirements" }
        }
    }
    if (!(Test-Path -LiteralPath $Python -PathType Leaf)) { throw "虚拟环境未生成解释器：$Python" }
    $manifests = @($pyproject, $requirements, $lock) | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf }
    $fingerprint = ($manifests | ForEach-Object { "$_`t$((Get-FileHash -LiteralPath $_ -Algorithm SHA256).Hash)" }) -join "`n"
    Set-Content -LiteralPath $stamp -Value $fingerprint -NoNewline -Encoding utf8
}
if ($Script) {
    & $Python $Script
    exit $LASTEXITCODE
}
