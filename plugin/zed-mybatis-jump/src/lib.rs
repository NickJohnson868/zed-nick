use zed::settings::LspSettings;
use zed_extension_api as zed;

struct MyBatisJump;

impl zed::Extension for MyBatisJump {
    fn new() -> Self {
        Self
    }

    fn language_server_command(
        &mut self,
        _id: &zed::LanguageServerId,
        worktree: &zed::Worktree,
    ) -> zed::Result<zed::Command> {
        if let Some(binary) = LspSettings::for_worktree("mybatis-jump", worktree)?.binary {
            return Ok(zed::Command {
                command: binary.path.ok_or("缺少 MyBatis Jump 服务路径")?,
                args: binary.arguments.unwrap_or_default(),
                env: binary.env.unwrap_or_default().into_iter().collect(),
            });
        }
        let filename = match zed::current_platform().0 {
            zed::Os::Windows => "mybatis-jump-lsp.exe",
            _ => "mybatis-jump-lsp",
        };
        let bundled = std::env::current_dir()
            .map_err(|error| error.to_string())?
            .join("bin")
            .join(filename);
        let command = if bundled.is_file() {
            bundled.to_string_lossy().into_owned()
        } else {
            worktree.which(filename).ok_or_else(|| {
                format!("未安装当前平台的 {filename}；请运行插件安装脚本，或配置 lsp.mybatis-jump.binary.path")
            })?
        };
        Ok(zed::Command {
            command,
            args: Vec::new(),
            env: Vec::new(),
        })
    }
}

zed::register_extension!(MyBatisJump);
