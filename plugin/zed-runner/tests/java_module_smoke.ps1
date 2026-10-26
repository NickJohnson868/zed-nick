#requires -Version 7.0
param([Parameter(Mandatory=$true)][string]$JavaHome)
$ErrorActionPreference = 'Stop'
$root = Join-Path ([IO.Path]::GetTempPath()) ("zed 模块 🧪 ' smoke " + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path "$root/.zed","$root/src/main/java" -Force | Out-Null
Set-Content -LiteralPath "$root/pom.xml" -Value '<project />'
Set-Content -LiteralPath "$root/src/main/java/Main.java" -Value 'public class Main { public static void main(String[] args) { System.out.println("MODULE_OK " + args[0]); } }'
$maven = Join-Path $root 'maven stub.ps1'
Set-Content -LiteralPath $maven -Value @'
$ErrorActionPreference = 'Stop'
$javac = Join-Path $env:JAVA_HOME $(if ($IsWindows) { 'bin/javac.exe' } else { 'bin/javac' })
New-Item -ItemType Directory -Path 'target/classes' -Force | Out-Null
& $javac -encoding UTF-8 -d target/classes src/main/java/Main.java
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
[IO.File]::WriteAllText((Join-Path (Get-Location) 'target/zed-run-classpath.txt'), '')
Add-Content -LiteralPath 'target/build-count.txt' -Value 'build'
'@
@{environments=@{java=@{runtime=$JavaHome;build_tool=$maven;args=@("quote ' 中文 argument")}}} | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath "$root/.zed/run-environments.json"
$provider = Join-Path (Split-Path $PSScriptRoot -Parent) 'run-provider.ps1'
$discovered = & $provider -Root $root | ConvertFrom-Json
$entry = $discovered.entries | Where-Object language -eq 'java' | Select-Object -First 1
if (!$entry) { throw "No module entry: $($discovered.warnings -join '; ')" }
$runner = Join-Path (Split-Path $PSScriptRoot -Parent) 'scripts/run_java.ps1'
foreach ($iteration in 1..2) {
    $env:ZED_JAVA_ENTRY = $entry.task.env.ZED_JAVA_ENTRY
    $output = & (Get-Process -Id $PID).Path -NoProfile -File $runner 2>&1 | Out-String
    if ($LASTEXITCODE -ne 0 -or !$output.Contains("MODULE_OK quote ' 中文 argument")) { throw "Module launch failed: $output" }
}
if (@(Get-Content -LiteralPath "$root/target/build-count.txt").Count -ne 1) { throw 'Unchanged module was rebuilt' }
Remove-Item Env:ZED_JAVA_ENTRY
Write-Host 'Passed: Java module launch, Unicode argument file, platform classpath, cached second launch.'
Write-Host "Temporary fixture: $root"

exit 0
