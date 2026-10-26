#requires -Version 7.0
$ErrorActionPreference = 'Stop'
$root = Join-Path ([IO.Path]::GetTempPath()) ("zed maven 中文 ' " + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path "$root/.zed","$root/service" -Force | Out-Null
Set-Content -LiteralPath "$root/pom.xml" '<project />'
Set-Content -LiteralPath "$root/service/pom.xml" '<project />'
$capture = Join-Path $root 'arguments.json'
$tool = Join-Path $root "fake maven ' executable.ps1"
Set-Content -LiteralPath $tool -Value @'
@{ arguments=@($args);cwd=(Get-Location).Path;marker=$env:MAVEN_MARKER } | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $env:MAVEN_CAPTURE
exit ([int]$env:MAVEN_EXIT)
'@
$settings = @{environments=@{java=@{build_tool=$tool;env=@{MAVEN_CAPTURE=$capture;MAVEN_EXIT='0';MAVEN_MARKER='configured'}}}}
$settings | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath "$root/.zed/run-environments.json"
$runner = Join-Path (Split-Path $PSScriptRoot -Parent) 'scripts/run_maven.ps1'
$shell = (Get-Process -Id $PID).Path
& $shell -NoProfile -File $runner -Root $root -Pom "$root/service/pom.xml" -ReactorPom "$root/pom.xml" -Goals 'clean,verify' -Profiles 'prod,!dev' -SkipTests -Offline -AlsoMake
if ($LASTEXITCODE -ne 0) { throw 'Maven task failed' }
$record = Get-Content -LiteralPath $capture -Raw | ConvertFrom-Json
foreach ($argument in @('-f',(Resolve-Path -LiteralPath (Join-Path $root 'pom.xml')).ProviderPath,'-pl','service','-am','-DskipTests','-o','-P','prod,!dev','clean','verify')) {
    if ($record.arguments -notcontains $argument) { throw "Lost Maven argument: $argument" }
}
if ($record.marker -ne 'configured' -or (Split-Path $record.cwd -Leaf) -ne (Split-Path $root -Leaf)) { throw 'Maven environment or reactor directory lost' }
$settings.environments.java.env.MAVEN_EXIT = '7'
$settings | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath "$root/.zed/run-environments.json"
& $shell -NoProfile -File $runner -Root $root -Pom "$root/service/pom.xml" -Goals 'validate'
if ($LASTEXITCODE -ne 7) { throw 'Maven failure exit code was lost' }
$settings.environments.java.build_tool = ''
$settings | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath "$root/.zed/run-environments.json"
if ($IsWindows) {
    Set-Content -LiteralPath "$root/mvnw.cmd" -Value '@echo WRAPPER_OK> "%MAVEN_CAPTURE%"'
} else {
    Set-Content -LiteralPath "$root/mvnw" -Value 'printf WRAPPER_OK > "$MAVEN_CAPTURE"'
}
& $shell -NoProfile -File $runner -Root $root -Pom "$root/pom.xml" -Goals 'validate'
if ($LASTEXITCODE -ne 0 -or (Get-Content -LiteralPath $capture -Raw).Trim() -ne 'WRAPPER_OK') { throw 'Project Maven wrapper was not used' }
Write-Host 'Passed: Maven module reactor arguments, profiles, flags, quoted paths, environment, failure exit code and native wrapper.'
exit 0
