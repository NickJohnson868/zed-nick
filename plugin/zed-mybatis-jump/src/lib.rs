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
        let binary = LspSettings::for_worktree("mybatis-jump", worktree)?
            .binary
            .ok_or("请配置 lsp.mybatis-jump.binary.path，指向 mybatis-jump-lsp.exe")?;
        Ok(zed::Command {
            command: binary.path.ok_or("缺少 MyBatis Jump 服务路径")?,
            args: binary.arguments.unwrap_or_default(),
            env: binary.env.unwrap_or_default().into_iter().collect(),
        })
    }
}

zed::register_extension!(MyBatisJump);
