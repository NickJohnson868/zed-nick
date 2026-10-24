# Windows 绿色版

双击项目根目录的 `build-portable.bat` 即可，或在项目根目录执行：

```powershell
.\build-portable.bat
```

默认 8 个并行任务，使用现有 Cargo 缓存，不执行 clean。BAT 调用同目录的 PowerShell 脚本并透传参数；需要 PowerShell 7、Rust、Visual Studio C++/Windows SDK 和 Spectre 库。先编译 GUI，再单独编译 CLI，成功后才组装目录。脚本优先使用本机 `Documents\tools\rust` 环境及 VS 2026 附带的 CMake，不修改系统环境变量。

交付物为项目根目录的 `Zed-Nick-Portable.zip`。解压后直接双击其中的 `Zed.exe`。BAT 仅用于编译打包，运行 Zed 不需要 CMD 或 PowerShell 启动器。脚本默认在 `.local-portable-build` 目录组装，不再使用 `release` 作为交付目录。ZIP 中的 `portable-data/` 只是空目录标记：Zed 靠它判断要把数据写在程序旁边，首次启动时才生成设置、数据库和缓存。ZIP 不包含本机的设置、登录信息、插件、语言服务、日志或项目数据。

默认通用打包不会安装个人插件，也不会复用已有数据的组装目录。如果 `.local-portable-build/portable-data` 已有内容，脚本会报错并要求改用新的空 `-OutputDirectory`，不会删除旧数据。ZIP 写完后还会验证其中的 `portable-data/` 确实为空。

首次迁移会排除崩溃处理通信文件等临时 socket，保留开发扩展的目录链接，并使用原子目录重命名完成迁移。AMD AGS 6.3.0 依赖优先从已有 Zed 获取，缺少时从上游固定版本 SDK 下载；OpenConsole 从构建输出查找。

只有明确传入 `-ImportProfile` 时，脚本才把当前账户 `%APPDATA%\Zed` 的配置和 `%LOCALAPPDATA%\Zed` 的数据复制到指定组装目录。此时先关闭所有 Zed 窗口；原目录保留，迁移失败时不把半成品当作完整配置。仓库根目录的 ZIP 始终保持空白配置。更新已解压的绿色版时，保留它自己的 `portable-data`。

```powershell
# 默认生成空白配置 ZIP，适合复制到另一台电脑
.\build-portable.bat

# 默认组装目录曾保存过个人数据时，改用新的空目录
.\build-portable.bat -OutputDirectory C:\Temp\Zed-Nick-Generic

# 同时将当前个人配置导入指定组装目录；ZIP 仍保持空白
.\build-portable.bat -OutputDirectory C:\Tools\Zed-Nick -ImportProfile

# 仅组装已成功编译过的本版本产物
.\build-portable.bat -SkipBuild
```

`-SkipBuild` 不检查已有二进制是否包含最新源码；首次构建或修改代码后不要使用。项目的 JDK、Python、Maven 等工具目录仍需在目标电脑存在或重新配置，绿色版不捆绑这些运行环境。

## 运行入口

顶栏下拉框选择项目入口，运行时按钮变为重启，停止按钮只在选中入口运行时可用。切换到另一语言的文件时，优先选择对应文件或语言的入口；可以在下拉框手动改选。扫描使用 Zed 的项目文件列表，跳过忽略文件、外部符号链接及常见依赖/输出目录，在后台读取文件，不启动语言服务或常驻扫描进程。新增入口后可点击菜单中的重新扫描。

初版识别范围：

| 语言 | 入口与运行方式 |
| --- | --- |
| Java | 常见 `public static void main(String[]/String...)`；所选 Maven 模块及依赖增量 compile，再以 target/classes 与依赖 classpath 运行 Main；无 Maven 时使用 JDK 源文件启动 |
| Python | main guard 或 main.py、app.py、manage.py、__main__.py；优先项目 .venv，再使用配置解释器 |
| JavaScript / TypeScript | 各 package.json 的 scripts，使用对应目录执行包管理器 run |
| Rust | 有 src/main.rs 的 Cargo 包，cargo run；自定义多个 bin 可手动补充配置 |
| Go | package main 中的 func main，按所在包 go run |
| .NET | 可执行或 Web csproj，dotnet run --project |

这不是完整的 AST/语言服务入口索引；特殊签名、Gradle、自定义构建系统及依赖多个源文件的无构建工具 Java 项目，使用手动入口覆盖。没有自动推断任意框架的启动参数。

识别到 Java main 时，使用 Main 类启动配置，不再把旧的 primary-run 打包任务绑定为默认 Java 入口；只有无法识别 Java 入口时才保留该任务作为回退。唯一的 Spring Boot main 会优先展示。Java 运行器读取项目 `.zed/run-config.json` 中已有的 env，保留原密钥；运行环境中显式配置的同名变量优先，私密值不写入公开源码或日志。

下拉框的“编辑运行环境”可创建/打开项目 `.zed/run-environments.json`。修改后重新扫描，例如：

```json
{
  "environments": {
    "java": {
      "runtime": "C:/Users/62453/Documents/tools/jdk/bellsoft25-lite",
      "build_tool": "C:/Users/62453/Documents/tools/maven/mvnd1.0.6/bin/mvnd.exe",
      "env": {},
      "args": [],
      "jvm_args": []
    },
    "python": {
      "runtime": "C:/Python/python.exe",
      "env": {"PYTHONUTF8": "1"},
      "args": []
    },
    "javascript": {"runtime": "npm", "env": {}},
    "rust": {"runtime": "cargo", "env": {}},
    "go": {"runtime": "go", "env": {}},
    "csharp": {"runtime": "dotnet", "env": {}}
  },
  "entries": []
}
```

`java.runtime` 可为 JDK 目录或 java.exe 路径；其他 runtime 填可执行文件。`env` 仅作用于相应任务，不修改全局环境。`java.args` 和 `python.args` 为应用参数，`java.jvm_args` 为 JVM 参数。JavaScript 的 runtime 是 npm/pnpm 等包管理器。配置为严格 JSON，字段名拼错会显示配置错误，避免悄悄使用错误运行环境。

手动入口使用 `entries`，`task` 沿用 Zed TaskTemplate 格式。`id` 与扫描得到的相对文件路径相同时可覆盖自动入口；`language` 使用上表示例中的小写语言键，`file` 为对应源文件相对路径。示例：

```json
{
  "environments": {"python": {"env": {"PYTHONUTF8": "1"}}},
  "entries": [{
    "id": "api-server",
    "language": "python",
    "file": "server.py",
    "task": {
      "label": "Python API",
      "command": "python",
      "args": ["-m", "uvicorn", "server:app"],
      "save": "all"
    }
  }]
}
```

## 项目树收起行为

默认开启，鼠标收起和键盘收起都清除该目录及后代目录的展开标记，其他目录保留。恢复旧行为：

```json
{"project_panel": {"collapse_children_on_collapse": false}}
```

设置页名称：**Collapse Children With Parent**。

## 本轮验证状态

2026-10-23 的历史验证：当时已用根目录 build-portable.bat、8 个并行任务完成 GUI 与 CLI 的 release-fast 构建和绿色版组装，BAT 返回 0。当时的 release/Zed.exe 与 GUI 构建产物、release/bin/zed.exe 与 CLI 构建产物的 SHA-256 一致；个人设置复制一致，稳定版数据库存在，MyBatis 开发扩展链接与 WASM 校验通过。这个旧目录不再是当前交付位置。


## Java 运行前增量构建优化

参考 IDEA 的 Application 配置：Main 类、模块 classpath、JDK、JVM 参数、程序参数和环境变量共同组成运行配置。启动前使用 Maven `compile dependency:build-classpath`，只构建 `-pl` 指定模块及 `-am` 依赖；不执行 package、install 或 clean。这里复用 Maven 增量编译，并未实现 IDEA 的 JPS 编译器。

缓存记录保存在启动模块的 target/zed-run-state.json。源码/资源、POM、Maven 配置、私密运行配置、工具链和编译输出未变化时直接启动，不再调用 Maven；变化或输出缺失时重新增量编译。无法确认输入完整性时不跳过编译。编译失败或构建期间输入变化时不启动旧代码。只在启动前检查，没有常驻扫描进程。

多模块依赖使用 target/classes，Java classpath 参数文件按 Windows 系统 ANSI 编码写入，程序参数直接传递，避免中文路径损坏及 Windows 命令行过长。

2026-10-23 验证：真实项目 compile + classpath 成功，42 个工作区依赖使用 classes 目录；连续缓存检查均跳过 Maven，优化后的单次检查约 4.2 秒，未启动业务服务。隔离样例通过中文目录/参数、环境变量、无修改重复启动、有修改重新编译及编译失败禁止启动旧代码的验证。

2026-10-24 已以 8 个并行任务将该源码编译为 GUI 与 CLI，并完成绿色版组装。仓库根目录 ZIP 的 GUI/CLI 与构建产物哈希一致，CLI 返回提交 `c6b606c2fe900ec98f6c95540d0db7b03253c0a3`；ZIP 内含空白 `portable-data`，未包含本机个人数据。实际 Java 运行按钮仍需在界面中验收。
