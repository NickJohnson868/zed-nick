# Personal Run and Debug

ZedNick 通过原生 Rust 代码扫描 Java、Python、Go、npm、Rust 和 C# 入口，在工具栏选择后运行或调试。Windows、macOS、Linux 均不要求安装 PowerShell 7；运行各语言项目时复用该项目配置的工具。此功能需要配套的新 ZedNick 主程序和 0.4.0 插件，单独更新插件不能改变旧主程序的扫描方式。

插件安装目录为当前配置目录下的 extensions/installed/zed-runner。构建机使用仓库的 plugin/install.py 打包当前 OS/CPU 的 Java、MyBatis 原生程序及插件；最终使用者不需要安装 Python、Rust 或 PowerShell 来安装已经打包好的插件。

## 项目运行环境

编辑 `.zed/zed-runner.json`，工具路径分别在每台电脑配置，不复制 Windows 路径到 Mac/Linux。运行环境模板由原生代码发现本机工具生成：

- `java.runtime`：JDK 目录；Maven 项目另配 `java.build_tool` 为 Maven/MVND 文件或其安装目录。Java 运行器使用配置的 JDK，不另找脚本解释器。
- `python.runtime`：Python 可执行文件；`python.uv`：可选 uv。pyproject.toml 项目需要 uv；requirements.txt 项目需要 Python，可选用 uv 安装依赖，否则使用 Python 自身的 venv/pip。
- `go.runtime`：Go 可执行文件。
- `javascript.runtime`：npm 可执行文件。

各语言的 env 是其运行、依赖准备和调试环境变量。Java 的 args、jvm_args 分别是应用与 JVM 参数。配置包含私密环境变量，不得提交到 Git。

配置中的 entries 可以追加自定义任务，任务指定的工具由用户自己负责。自动入口使用系统自带的 Windows CMD 或 Unix /bin/sh，路径和参数分别转义。Python 依赖环境由配置的 Python/uv 准备，Go/npm 直接调用配置的工具。调试分别使用 Java、Debugpy、Delve 适配器，不再通过 PowerShell 构建前置任务。

Git 项目以 git ls-files --cached --others --exclude-standard 为扫描范围；非 Git 项目使用目录扫描并排除生成目录。同一配置文件中的 scan_directories、ignore_directories 控制扫描范围；白名单使用根目录相对路径，单个忽略目录名匹配任意层级。入口扫描不安装依赖、不运行项目代码。

## Spring Boot 开发热更新

设置 Java 运行环境的 `hot_reload: true`，重新扫描后选择“Java 热更新 · 主类”。项目需提供 dev-hot-reload Maven profile：所有模块的 build.directory 为 `${project.basedir}/target/dev-hot-reload`，启动模块提供 DevTools。原生扫描器调用 scripts/JavaRunner.java，只使用项目必需的 JDK 和 Maven。

保存后自动编译，成功才更新 DevTools 重启触发文件；编译失败时保留进程，修正并保存后重试。监听包含实际依赖模块，即使该模块不在入口扫描白名单内。修改 POM/依赖后重新运行入口；删除、移动类后先停止，再用同一 profile clean 后启动。

热更新会重建 Spring 上下文，定时任务、消费者和连接也会重新初始化，期间请求可能短暂中断。仅使用开发环境；Ctrl+C 停止本次启动的进程。

## 验证

```sh
python plugin/zed-runner/tests/native_smoke.py
```

测试使用隔离项目，覆盖真实 Maven/Java 启动、中文与带空格路径、应用参数、DevTools 重启、失败拦截及恢复，并检查 Python 环境和安装布局。GitHub 工作流按 Windows、macOS、Linux 分别运行；编译检查与运行验证分别报告。

## 配置迁移

新版本首次加载项目时，将旧 run-config.json、run-environments.json 和 runner-ignore.json 合并为 `.zed/zed-runner.json`，保留环境变量、参数和扫描规则。同名环境变量以语言运行环境配置为准。MyBatis 单独使用 `.zed/mybatis-jump.json`。

迁移前请先停止旧启动任务。旧文件会先备份到当前用户缓存目录的 `zed-nick/runner` 下，确认备份后才退役。旧生成任务、run.ps1 和 .run-state.json 一并归档；用户自行编写的其他 tasks.json 任务保留。已有统一配置时不覆盖它。Git 项目会将新的私密配置加入本地 info/exclude。

Java 行内任务由打包的原生 java-lsp-proxy 执行，运行、测试和清理语言服务缓存均不需要 PowerShell。Java/Python 辅助运行器只复用对应项目必需的 JDK/Python。构建机的可选 install.ps1 仅用于调用打包工具，不随插件作为运行依赖。
