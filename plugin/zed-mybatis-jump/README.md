# MyBatis Jump for Zed

本机 Zed 扩展，提供 Mapper Java 与 MyBatis XML 的双向定义跳转。
使用 Zed 原生 **F12 / Ctrl+点击**，不需要文本入口、自定义面板或修改 Zed 源码。

## 使用范围

- Java 接口的**方法声明名称** → XML 对应的 `select/insert/update/delete`。
- XML SQL 节点的 **`id` 属性值** → Java Mapper 方法声明。
- XML 根节点的 **`namespace` 属性值** → Java Mapper 接口声明。
- `resultMap` 引用、`resultMap extends` → 对应 `<resultMap id>`，支持逗号列表和跨 namespace 引用。
- `<include refid>` → 对应 `<sql id>`，支持跨 namespace 引用。
- `type/resultType/parameterType/javaType/ofType` 中的 Java 全限定名 → 项目源码中的类、record、接口、枚举或注解类型。
- 同文件 XML 定义优先读取当前缓冲区，未保存的 resultMap 和 SQL 片段也可定位。
- 当前不解析自定义 MyBatis 类型别名、依赖 JAR 或嵌套 Java 类型。
- 精确匹配接口全限定名与 `namespace`、方法名与 SQL `id`，不依赖 XML 文件名。
- 不处理 Service 中的方法调用、嵌套接口、跨接口继承关系或反向跳转到未保存的 Java 修改。
- 若同一映射存在多个定义，交给 Zed 展示候选位置。

## 与现有 Java 支持并行

扩展只注册名为 `mybatis-jump` 的补充语言服务，绑定已有 Java / XML 语言，
不替换 XML 语法、JDTLS、快捷键或格式化；新增的补全、诊断和配色只处理 MyBatis XML。
普通 Java 类型、方法调用、没有 XML 映射的方法返回空结果，原有 Java 服务继续提供结果。
Zed 会合并定义位置，所以重复映射或多个服务返回不同位置时可能展示多个候选。

服务是 Rust 原生程序，不使用 JVM；仅保留已打开文件的文本，按跳转请求扫描，
不在后台建立全仓 Java 索引，不启动常驻扫描线程。XML 外部 DTD 不联网读取。
扫描跳过隐藏目录、target、build、node_modules、temp、bin、out 和符号链接，
单文件上限 4 MiB。重命名、保存后的 XML 和磁盘 Java 修改会在下次查询时读取。

## 构建

```powershell
cd C:\Users\62453\OneDrive\工具软件\zed-mybatis-jump
.\build.ps1
python tests/smoke.py
```

使用现有 `C:\Users\62453\Documents\tools\rust` 环境。输出：

- `extension.wasm`：官方 SDK 扩展入口。
- `bin/mybatis-jump-lsp.exe`：原生 stdio LSP 服务。

重新部署原生程序前，先在 Zed 停止 **MyBatis Jump** 服务，避免 Windows 文件占用；
构建后重启该服务即可，无需停止 JDTLS。

## 本机安装

在 Zed 命令面板执行 `zed: install dev extension`，选择本项目目录。
当前开发扩展目录已链接到本项目。

在 Zed 用户配置合并以下字段，保留已有 Java / XML 语言服务：

```json
{
  "lsp": {
    "mybatis-jump": {
      "binary": {
        "path": "C:/Users/62453/OneDrive/工具软件/zed-mybatis-jump/bin/mybatis-jump-lsp.exe"
      }
    }
  },
  "languages": {
    "Java": {
      "language_servers": ["jdtls", "mybatis-jump", "..."]
    },
    "XML": {
      "language_servers": ["mybatis-jump", "..."]
    }
  }
}
```

Java 语法和 JDTLS 由原 Java 扩展提供；XML 语法由 XML 扩展提供。
安装或修改扩展清单后执行 `zed: reload extensions`。

## 验证

`build.ps1` 包含 Rust 格式检查、规则测试、原生服务 Clippy 检查及发布编译。
`tests/smoke.py` 通过真实 LSP 管道验证双向位置、实时修改、非 Mapper 空结果、
生成目录排除和进程退出，并读取 ai-manager-server 的 ServerClockMapper 样例，
不修改业务仓库文件；启用数据库配置的项目会触发只读 SQL 校验。

## PostgreSQL 字段检查与补全（0.2）

在项目 `.zed/mybatis-jump.json` 中配置连接，通过 `active_connection` 选择。
完整模板见 `mybatis-jump.example.json`。当前 ai-manager-server 已配置 `local-dev`，
引用本地 `application-dev.yaml` 的 `spring.datasource`；不复制或记录其中的密码。
也支持直接配置主机、数据库、用户名并用 `password_env` 引用环境变量。
YAML 的 `${ENV:default}`、JDBC `currentSchema` 和显式 `search_path` 均可识别。
修改连接配置并保存后约 1 秒内重新验证已打开的 XML。

- **红色波浪线**：不存在的字段、表/关系、语法错误及无法确定归属的同名字段。
- **字段 / 表补全**：XML 静态 SQL 内输入 `别名.` 或按 Ctrl+Space；表来自配置的 `schemas`。
- **保留原有跳转**：Java 和 XML 的 F12 / Ctrl+点击继续使用，普通 Java 补全返回空结果。
- **动态 SQL**：含 if、foreach、choose、where、set、include 等子标签，或 `${...}` 的整条语句暂跳过；
  不把猜测的展开结果标成错误。`#{...}` 替换成 PostgreSQL 参数，CDATA 和 XML 转义保持定位。

字段和语法检查使用 PostgreSQL 扩展查询协议的 **Parse / Describe**（Rust `Client::prepare`）。
这可以按数据库的真实对象、别名和 CTE 规则判断字段，但**不发送 Mapper SQL 的 Bind / Execute**。
实际执行的查询仅是插件固定的 pg_catalog 元数据读取、search_path 设置和只读状态检查。
连接默认事务只读，语句超时 3 秒、锁超时 1 秒；数据库不可用时清除本插件的字段错误并报告连接状态，
不会把连接失败当成字段不存在。

服务保留最多一个数据库连接，空闲约 10 秒关闭；表字段元数据仅内存缓存，默认 300 秒。
编辑防抖约 450 毫秒，数据库工作在单个后台线程，旧版本诊断不会覆盖新内容，
不启动额外 JVM、不缓存业务数据、不在后台扫描全仓 SQL。
每个 SQL 块最多 256 KiB，每次最多分析 100 条静态语句；超出部分不报告推测错误。
SQL 检查需要连接可用，未配置连接时原有跳转仍可独立使用。

### 本机只读验证

```powershell
.\bin\mybatis-jump-lsp.exe --check-db C:\Users\62453\Documents\projects\ai-manager-server
python tests/postgres_smoke.py
```

验证覆盖真实 PostgreSQL 字段/表错误、静态语法、CTE、CDATA、XML 红线位置、
修复后诊断清除、别名字段补全及 Java 隔离。自检只读取元数据，不执行 Mapper SQL。

### SQL 配色

XML 的 `semantic_tokens` 设置为 `combined` 后，插件为 SQL 正文提供语义高亮，保留 XML 标签原有颜色。使用独立的 mybatis token 类型，避免改变 Java 配色。默认参考 IDEA Darcula：关键字橙色、标识符紫色、字符串绿色、数字蓝色、注释灰色。高亮不依赖数据库连接，不代表数据库连接成功；动态 SQL 的文本也会高亮，但仍跳过字段诊断以避免误报。

配色配置模板：`mybatis-semantic-colors.example.json`。本机已合并到 Zed 用户配置。
本机业务 schema 已核实为 `el、jd、mt、o2o、system`，共 195 张表/视图、4028 个字段。

`python tests/xml_navigation_smoke.py` 验证 XML 引用、跨 namespace、未保存定义和 Java 类型跳转。

### property 与响应速度

`id/result/association/collection` 的 `property` 可跳到所属 Java 类声明的字段或 record 组件（包括 Lombok 字段）。嵌套 association/collection 使用显式 `javaType/ofType` 确认所属类型，暂不推测嵌套点路径和继承字段。
XML 定义返回支持 LocationLink 的客户端所需的 originSelectionRange，定位属性值的完整可点击范围。
文件路径索引按项目复用，最长 30 秒刷新；打开/保存的新源码立即加入索引。同文件 resultMap/sql 直接使用当前缓冲区，不扫描项目目录。不保留全仓 Java AST 或业务数据。
实测本机项目首次索引约 140ms，重复类型查询约 0.5ms、字段查询约 0.9ms、同文件 XML 引用约 0.06ms。这是 LSP 管道耗时，不包含编辑器自己的鼠标悬停延迟。
`python tests/property_smoke.py` 验证字段、record 组件、嵌套显式类型、排除局部变量和链接源范围。

## 映射检查、属性补全、参数检查

- **resultMap 属性检查**：`property` 对照保存后的 Java 源码字段、record 组件、bean getter/setter 和可解析的父类属性；拼错显示红色波浪线。无法解析的类型、泛型或外部父类不推测缺失。
- **resultMap 返回列检查**：使用 PostgreSQL Parse/Describe 的实际结果列名（含 SQL 别名、通配列），不是直接比数据库表字段。支持本文件固定的 `<include refid>`，只在所有引用语句均能静态确定时检查；多条语句返回列取并集，避免可选映射误报。嵌套 map、extends、动态 SQL、未知或循环 include 暂不检查返回列。
- **XML 补全**：属性值内按 Ctrl+Space，或输入引号/点触发；`property` 提供所属 Java 属性，`resultMap/extends/refid` 提供映射/片段（含完整 namespace 引用），`type/resultType/parameterType/javaType/ofType` 提供项目 Java 类型。候选最多 100 项。
- **Mapper 参数检查**：静态 SQL 中的 `#{...}` 对照 Mapper 方法的 `@Param`、多参数通用名称和 DTO 路径。单一基础类型参数允许任意占位符名称；Map、无法确定的类型、重载方法和动态分支不推测报错。静态 include 查询支持检查 SQL 正文中的参数，片段自身共享参数暂不检查。

Java 属性补全与本地参数检查不依赖数据库；数据库离线时继续工作。保存 Java 后自动重查已打开的 XML；Java 分析使用保存后的源码，XML 使用当前编辑缓冲区。
仅缓存最多 128 个 Java 类型的字段/方法元数据，不保留其 AST；跨 namespace 候选缓存最多 32 项，最长 30 秒，XML 保存时失效。

验证入口：`python tests/features_smoke.py`（离线 Java 分析及真实数据库返回列）、`python tests/repository_mapping_smoke.py`（真实 ImagePromptLibraryMapper 的 style_body1 诊断）。测试不修改业务源码，不执行 Mapper SQL。

源码已迁移至 OneDrive 的工具软件目录。默认 Rust 编译缓存位于本机 `Documents/tools/rust/targets/zed-mybatis-jump`，不放进 OneDrive。项目集成测试默认使用 `C:/Users/62453/Documents/projects/ai-manager-server`；可用 `MYBATIS_PROJECT_ROOT` 覆盖。

## SQL 参数引用跳转

SQL 正文中的 `#{name}` / `${name}` 支持 F12 和 Zed 原生 Ctrl+点击定义跳转；服务向客户端返回参数表达式的源范围。

- 语句自己的 `<bind name>`，以及通过固定 `<include refid>` 引入的 bind：跳到 XML 的 name 属性；支持本文件和跨 namespace 片段，循环/动态 refid 不推测。
- foreach 内部的 item/index：跳到最近一层 foreach 的声明属性，局部变量优先，不泄漏到循环外。
- Mapper 的 `@Param` 别名、方法参数名、paramN/argN：跳到 Java 参数声明。
- `#{query.field}`、单 DTO 的 `#{field}`：在可解析的项目类型中跳到字段。未知类型、Map、重载方法不猜测；单一基础类型的占位符可跳到该参数。

截图场景 `O2oOperationTaskMapper.xml` 中的 sourceProductCodes、operationUpdateStoreCode、operationTargetProduct 均来自 OperationTaskCommonBinds，定位其 bind，而不是 Java 参数。实测重复请求约 0.2～0.3ms，Java 参数约 2.3ms；不包含编辑器自身悬停延迟。

导航不执行 SQL，也不因此开启动态 SQL 字段诊断。foreach 元素对象的子字段、复杂 OGNL 和继承字段的导航暂不推测。验证入口：`python tests/parameter_navigation_smoke.py`，包含截图原始 Mapper、跨 namespace bind、DTO、@Param、循环作用域和链接源范围。实际鼠标反馈仍需在 Zed 中验证，协议测试不能替代界面验证。
