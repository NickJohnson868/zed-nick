#requires -Version 7.0
param([Parameter(Mandatory=$true)][string]$JavaHome)
$ErrorActionPreference = 'Stop'
if (!$IsWindows) { throw 'This test requires Windows PowerShell 5.1' }
$shell = Join-Path $env:SystemRoot 'System32/WindowsPowerShell/v1.0/powershell.exe'
$root = Join-Path ([IO.Path]::GetTempPath()) ("zed ps51 中文 ' " + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $root -Force | Out-Null
$fixture = Join-Path $root 'compatibility.ps1'
$platform = (Resolve-Path -LiteralPath "$PSScriptRoot/../scripts/platform.ps1").ProviderPath
$body = '. ' + "'" + $platform.Replace("'", "''") + "'`n" + @'
$ErrorActionPreference = 'Stop'
if ($PSVersionTable.PSVersion.Major -ne 5) { throw 'Expected Windows PowerShell 5.1' }
if (!(Test-RunnerAbsolutePath 'C:\中文\x') -or !(Test-RunnerAbsolutePath '\\server\share\x')) { throw 'Absolute path rejected' }
if ((Test-RunnerAbsolutePath 'C:relative') -or (Test-RunnerAbsolutePath '\relative')) { throw 'Relative path accepted' }
if ((Get-RunnerRelativePath 'C:\项目' 'C:\项目\模块\pom.xml') -ne '模块\pom.xml') { throw 'Unicode relative path lost' }
if ((Get-RunnerRelativePath 'C:\项目' 'D:\模块') -ne 'D:\模块') { throw 'Cross-drive path lost' }
$bytes = [Text.Encoding]::UTF8.GetBytes('abc')
if ((Get-RunnerSha256 $bytes) -ne 'BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD') { throw 'SHA256 mismatch' }
$value = @{name='中文';args=@('a','b');nested=@{enabled=$true}} | ConvertTo-RunnerJson | ConvertFrom-Json
$converted = ConvertTo-RunnerHashtable $value
if ($converted.name -ne '中文' -or $converted.args.Count -ne 2 -or !$converted.nested.enabled) { throw 'JSON conversion lost data' }
Write-Output 'PS51_COMPATIBILITY_OK'
'@
[IO.File]::WriteAllText($fixture, $body, [Text.UTF8Encoding]::new($true))
& $shell -NoProfile -ExecutionPolicy Bypass -File $fixture
if ($LASTEXITCODE) { throw 'PowerShell 5.1 compatibility helpers failed' }
& $shell -NoProfile -ExecutionPolicy Bypass -File "$PSScriptRoot/scan_smoke.ps1"
if ($LASTEXITCODE) { throw 'PowerShell 5.1 scan failed' }
& "$PSScriptRoot/task_command_smoke.ps1" -JavaHome $JavaHome -RunnerShell $shell
if ($LASTEXITCODE) { throw 'PowerShell 5.1 generated tasks failed' }
& "$PSScriptRoot/java_module_smoke.ps1" -JavaHome $JavaHome -RunnerShell $shell
if ($LASTEXITCODE) { throw 'PowerShell 5.1 Java module failed' }
& "$PSScriptRoot/maven_smoke.ps1" -RunnerShell $shell
if ($LASTEXITCODE) { throw 'PowerShell 5.1 Maven task failed' }
