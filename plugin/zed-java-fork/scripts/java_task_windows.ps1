$ErrorActionPreference='Stop'
[Console]::InputEncoding=[Text.UTF8Encoding]::new($false)
[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false)
$OutputEncoding=[Console]::OutputEncoding
function Invoke-JavaTask([string]$Mode) {
  $root=(Get-Location).Path
  if($Mode -eq 'cache') {
    $removed=0
    foreach($base in @((Join-Path $env:LOCALAPPDATA 'Zed/extensions/work/java'),(Join-Path $env:APPDATA 'Zed/extensions/work/java'))) {
      if(!(Test-Path -LiteralPath $base)){continue}
      $resolved=[IO.Path]::GetFullPath($base).TrimEnd('\')
      foreach($directory in (Get-ChildItem -LiteralPath $base -Directory -Filter 'jdtls-*')) {
        if(!$directory.FullName.StartsWith($resolved+'\',[StringComparison]::OrdinalIgnoreCase) -or $directory.Attributes.HasFlag([IO.FileAttributes]::ReparsePoint)){throw 'Refusing external cache path'}
        Remove-Item -LiteralPath $directory.FullName -Recurse -Force
        $removed++
      }
    }
    Write-Host "Cleared $removed default JDTLS caches. Custom data directories must be managed separately."
    return
  }
  $file=$env:ZED_FILE
  $class=$env:ZED_CUSTOM_java_class_name
  if($env:ZED_CUSTOM_java_outer_class_name){$class=$env:ZED_CUSTOM_java_outer_class_name+'$'+$class}
  if($env:ZED_CUSTOM_java_package_name){$class=$env:ZED_CUSTOM_java_package_name+'.'+$class}
  $module=Split-Path $file -Parent
  while($module -and $module.StartsWith($root,[StringComparison]::OrdinalIgnoreCase) -and !(Test-Path -LiteralPath (Join-Path $module 'pom.xml'))) {
    if($module -eq $root){$module=$null;break}
    $module=Split-Path $module -Parent
  }
  if($module) {
    $reactor=if(Test-Path -LiteralPath (Join-Path $root 'pom.xml')){$root}else{$module}
    $wrapper=Join-Path $reactor 'mvnw.cmd'
    $maven=if(Test-Path -LiteralPath $wrapper){$wrapper}else{(Get-Command mvnd.exe,mvn.cmd,mvn -ErrorAction SilentlyContinue|Select-Object -First 1).Source}
    if(!$maven){throw 'Configure Maven/MVND or provide mvnw.cmd'}
    $options=@('-f',(Join-Path $reactor 'pom.xml'),'-B')
    if($module -ne $reactor){$relative=$module.Substring($reactor.Length).TrimStart('\','/').Replace('\','/');$options+=@('-pl',$relative,'-am')}
    if($Mode -ne 'main') {
      $selector=if($Mode -eq 'method'){$class+'#'+$env:ZED_CUSTOM_java_method_name}else{$class}
      $arguments=$options+@('test','-DfailIfNoTests=false','-Dsurefire.failIfNoSpecifiedTests=false')
      if($Mode -ne 'all'){$arguments+='-Dtest='+$selector}
      & $maven @arguments
      if($LASTEXITCODE){throw 'Maven tests failed'}
      return
    }
    $test=$file.Replace('\','/').Contains('/src/test/')
    $goal=if($test){'test-compile'}else{'compile'}
    $scope=if($test){'test'}else{'runtime'}
    $arguments=$options+@('-DskipTests',$goal,'dependency:build-classpath','-Dmdep.outputFile=target/zed-java-classpath.txt',('-DincludeScope='+$scope),'-Dmdep.regenerateFile=true')
    & $maven @arguments
    if($LASTEXITCODE){throw 'Maven compilation failed'}
    $classes=@((Join-Path $module 'target/classes'))
    if($test){$classes=@((Join-Path $module 'target/test-classes'))+$classes}
    $classes+=((Get-Content -LiteralPath (Join-Path $module 'target/zed-java-classpath.txt') -Raw).Trim()-split ';')
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $jar=Join-Path $module 'target/zed-java-main-classpath.jar'
    if(Test-Path -LiteralPath $jar){Remove-Item -LiteralPath $jar}
    $urls=@($classes|Where-Object{$_}|ForEach-Object{$path=[IO.Path]::GetFullPath($_);if([IO.Directory]::Exists($path)){$path=$path.TrimEnd('\')+'\'};([Uri]::new($path)).AbsoluteUri})
    $line='Class-Path: '+($urls-join ' ')
    $lines=@('Manifest-Version: 1.0')
    while($line.Length -gt 70){$lines+=$line.Substring(0,70);$line=' '+$line.Substring(70)}
    $lines+=$line
    $archive=[IO.Compression.ZipFile]::Open($jar,[IO.Compression.ZipArchiveMode]::Create)
    try{$writer=[IO.StreamWriter]::new($archive.CreateEntry('META-INF/MANIFEST.MF').Open(),[Text.UTF8Encoding]::new($false));try{$writer.Write(($lines-join "`r`n")+"`r`n`r`n")}finally{$writer.Dispose()}}finally{$archive.Dispose()}
    & java '-Dfile.encoding=UTF-8' '-Dstdout.encoding=UTF-8' '-cp' $jar $class
    if($LASTEXITCODE){throw 'Java application failed'}
    return
  }
  if((Test-Path -LiteralPath (Join-Path $root 'build.gradle')) -or (Test-Path -LiteralPath (Join-Path $root 'build.gradle.kts')) -or (Test-Path -LiteralPath (Join-Path $root 'settings.gradle')) -or (Test-Path -LiteralPath (Join-Path $root 'settings.gradle.kts'))) {
    $module=Split-Path $file -Parent
    while($module -and $module -ne $root -and !(Test-Path -LiteralPath (Join-Path $module 'build.gradle')) -and !(Test-Path -LiteralPath (Join-Path $module 'build.gradle.kts'))){$module=Split-Path $module -Parent}
    $prefix=if($module -and $module -ne $root){':'+$module.Substring($root.Length).TrimStart('\','/').Replace('\',':').Replace('/',':')}else{''}
    $gradle=if(Test-Path -LiteralPath (Join-Path $root 'gradlew.bat')){Join-Path $root 'gradlew.bat'}else{'gradle'}
    $arguments=if($Mode -eq 'main'){@(($prefix+':run'),('-PmainClass='+$class))}else{@(($prefix+':test'))}
    if($Mode -in @('method','class')){$selector=if($Mode -eq 'method'){$class+'.'+$env:ZED_CUSTOM_java_method_name}else{$class};$arguments+=@('--tests',$selector)}
    & $gradle @arguments
    if($LASTEXITCODE){throw 'Gradle task failed'}
    return
  }
  if($Mode -ne 'main'){throw 'Tests require a Maven or Gradle project'}
  & java $file
  if($LASTEXITCODE){throw 'Java source launch failed'}
}
