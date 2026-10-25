# 个人运行与调试入口

此目录位于 `zed-nick/plugin/zed-runner`，是 Zed-Nick 的本地开发扩展。右上角的入口选择、运行、调试、停止按钮由 Zed-Nick 提供；具体扫描规则、运行命令和调试配置由本目录的 `run-provider.ps1` 返回。修改脚本后，在入口菜单选“重新扫描入口 / 重载运行环境”，无需重新编译 Zed。

安装到绿色版时，将本目录放到绿色版的 `portable-data/extensions/installed/zed-runner`。开发机上也可以把该目录链接到本目录，修改脚本后在入口菜单选择“重新扫描入口 / 重载运行环境”。

```powershell
$installed = 'C:\Users\62453\Documents\tools\Zed-Nick\portable-data\extensions\installed\zed-runner'
$source = 'C:\Users\62453\Documents\projects\zed-nick\plugin\zed-runner'
New-Item -ItemType Junction -Path $installed -Target $source
```

插件读取项目的 `.zed/run-environments.json`。在入口菜单中选择“编辑运行环境”时，首次创建配置会运行 `config-template.ps1`，从已配置的环境变量和工具路径生成绝对路径；自动填写不到的路径需要手动补齐。`java.runtime` 是包含 `bin/java.exe` 的 JDK 目录，`java.build_tool` 是 `mvn.cmd` 或 `mvnd.exe` 的绝对路径，`python.runtime`、`python.uv`、`go.runtime`、`javascript.runtime` 都是可执行文件的绝对路径。运行时不回退到全局 PATH。`env` 中可配置环境变量，并传递给运行、调试和依赖安装任务。Java 运行任务继续在启动时读取 `.zed/run-config.json` 的私密环境变量；Java 调试配置也会读取该文件。不要把含密钥的配置提交到 Git。

Python 的 `pyproject.toml` 入口要求配置 `python.uv`，`requirements.txt` 入口要求配置 `python.runtime`；两种文件都有时两条路径都必须配置。没有依赖文件时也要求配置 `python.runtime`。运行和调试优先使用入口附近的 `.venv/Scripts/python.exe`，缺失时由依赖安装任务创建项目内虚拟环境；没有所需工具路径的入口不扫描。`uv` 项目用 `uv sync`，`requirements.txt` 项目用 `uv pip` 或本地虚拟环境的 pip 安装依赖；依赖文件未变时复用安装结果。入口扫描本身不安装依赖。Go 入口要求 `go.runtime`，npm 入口要求 `javascript.runtime`。Java Main 要求 `java.runtime`；Maven 模块还要求 `java.build_tool`。

`run-provider.ps1 -Root <项目目录>` 向标准输出返回一个 JSON 对象：`version: 1`、`entries`、`warnings`。每个入口有 `id`、`language`、相对项目路径 `file`、`matching_extensions`、Zed `TaskTemplate` 格式的 `task`，以及可选的 Zed `DebugScenario` 格式的 `debug`。没有 `debug` 的入口不启用调试按钮。当前自动扫描 Java `main`、Python `__main__`、Go `main` 和 npm scripts；项目配置中的 `entries` 可以追加自定义入口。Go 使用 Zed 的 Delve 适配器，Python 使用 Debugpy，Java 使用已安装 Java 扩展的调试适配器。首次调试可能需要下载适配器，Java 还需要语言服务完成项目导入。

Git 项目通过 `git ls-files --cached --others --exclude-standard` 获取候选文件，自动遵守 `.gitignore` 和 Git 的其他忽略规则；即使结果为空也不会退回无过滤的目录遍历。非 Git 目录才使用内置目录扫描。Java 增量构建的输入指纹采用相同的候选文件范围。

项目可在 `.zed/runner-ignore.json` 中指定白名单与忽略名单，例如 `{"scan_directories":["ai-manager-app","script"],"ignore_directories":["tools"]}`。创建此文件后只扫描白名单目录；忽略名单只在白名单内生效。`scan_directories` 使用项目根目录相对路径，填 `"."` 表示整个项目。`ignore_directories` 中单个目录名（如 `tools`）匹配白名单内任意层级；相对路径（如 `script/generated`）只匹配项目根目录下的对应目录。Git 忽略规则始终生效。不创建此文件时沿用原有扫描范围。修改后在入口菜单选择“重新扫描入口 / 重载运行环境”。

可直接检查入口数量和调试支持，不打印环境变量：

```powershell
$result = & 'C:\Users\62453\Documents\projects\zed-nick\plugin\zed-runner\run-provider.ps1' -Root 'C:\Users\62453\Documents\projects\ai-manager-server' | ConvertFrom-Json
$result.entries | Group-Object language | Select-Object Name,Count
$result.entries | Where-Object { !$_.debug } | Select-Object id
```
