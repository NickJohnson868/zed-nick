# 个人运行与调试入口

此目录位于 `zed-nick/plugin/zed-runner`，是 Zed-Nick 的本地开发扩展。右上角的入口选择、运行、调试、停止按钮由 Zed-Nick 提供；具体扫描规则、运行命令和调试配置由本目录的 `run-provider.ps1` 返回。修改脚本后，在入口菜单选“重新扫描入口 / 重载运行环境”，无需重新编译 Zed。

安装到绿色版：

```powershell
$installed = 'C:\Users\62453\Documents\tools\Zed-Nick\portable-data\extensions\installed\zed-runner'
$source = 'C:\Users\62453\Documents\projects\zed-nick\plugin\zed-runner'
New-Item -ItemType Junction -Path $installed -Target $source
```

插件读取项目的 `.zed/run-environments.json`。从 `run-environments.template.json` 复制初始配置，再把 `java.runtime`、`java.build_tool`、`go.runtime` 等改为本机可执行文件的绝对路径。`env` 中可配置环境变量。Java 运行任务继续在启动时读取 `.zed/run-config.json` 的私密环境变量；Java 调试配置也会读取该文件。不要把含密钥的配置提交到 Git。

`run-provider.ps1 -Root <项目目录>` 向标准输出返回一个 JSON 对象：`version: 1`、`entries`、`warnings`。每个入口有 `id`、`language`、相对项目路径 `file`、`matching_extensions`、Zed `TaskTemplate` 格式的 `task`，以及可选的 Zed `DebugScenario` 格式的 `debug`。没有 `debug` 的入口不启用调试按钮。当前自动扫描 Java `main`、Python `__main__`、Go `main`、npm scripts、Cargo `src/main.rs` 和 `.csproj`；项目配置中的 `entries` 可以追加自定义入口或覆盖同 ID 入口。Go 使用 Zed 的 Delve 适配器，Python 使用 Debugpy，Java 使用已安装 Java 扩展的调试适配器。首次调试可能需要下载适配器，Java 还需要语言服务完成项目导入。

可直接检查入口数量和调试支持，不打印环境变量：

```powershell
$result = & 'C:\Users\62453\Documents\projects\zed-nick\plugin\zed-runner\run-provider.ps1' -Root 'C:\Users\62453\Documents\projects\ai-manager-server' | ConvertFrom-Json
$result.entries | Group-Object language | Select-Object Name,Count
$result.entries | Where-Object { !$_.debug } | Select-Object id
```
