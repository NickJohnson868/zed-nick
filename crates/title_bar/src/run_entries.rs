use anyhow::{Context as _, Result, ensure};
use serde::Deserialize;
use std::{
    fs,
    path::{Path, PathBuf},
};
use task::{DebugScenario, TaskTemplate};

pub const CONFIG_PATH: &str = ".zed/run-environments.json";
pub const RUN_TAG: &str = "toolbar-run";
const PROVIDER_SCRIPT: &str = "run-provider.ps1";
const TEMPLATE_FILE: &str = "run-environments.template.json";
const TEMPLATE_SCRIPT: &str = "config-template.ps1";

#[derive(Clone, Deserialize)]
pub struct RunEntry {
    pub id: String,
    pub language: String,
    pub file: PathBuf,
    #[serde(default)]
    pub matching_extensions: Vec<String>,
    pub task: TaskTemplate,
    #[serde(default)]
    pub debug: Option<DebugScenario>,
}

#[derive(Deserialize)]
struct ProviderOutput {
    version: u32,
    #[serde(default)]
    entries: Vec<RunEntry>,
    #[serde(default)]
    warnings: Vec<String>,
}

pub struct Discovery {
    pub entries: Vec<RunEntry>,
    pub warnings: Vec<String>,
}

fn providers() -> Result<Vec<PathBuf>> {
    let installed = paths::extensions_dir().join("installed");
    let mut providers = Vec::new();
    let directories = match fs::read_dir(&installed) {
        Ok(directories) => directories,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(providers),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("无法读取扩展目录：{}", installed.display()));
        }
    };
    for directory in directories {
        let directory = directory?.path();
        if directory.join("extension.toml").is_file() && directory.join(PROVIDER_SCRIPT).is_file() {
            providers.push(directory);
        }
    }
    providers.sort();
    Ok(providers)
}

pub fn config_template() -> Result<String> {
    for provider in providers()? {
        let generator = provider.join(TEMPLATE_SCRIPT);
        if generator.is_file() {
            let output = std::process::Command::new("pwsh")
                .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-File"])
                .arg(&generator)
                .output()
                .with_context(|| format!("无法运行配置模板：{}", generator.display()))?;
            ensure!(
                output.status.success(),
                "生成运行环境模板失败：{}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
            ensure!(output.stdout.len() <= 64 * 1024, "运行环境模板输出过大");
            serde_json::from_slice::<serde_json::Value>(&output.stdout)
                .context("运行环境模板不是有效 JSON")?;
            return String::from_utf8(output.stdout).context("运行环境模板不是 UTF-8 文本");
        }
        let template = provider.join(TEMPLATE_FILE);
        if template.is_file() {
            return fs::read_to_string(&template)
                .with_context(|| format!("无法读取运行环境模板：{}", template.display()));
        }
    }
    Ok("{\n  \"environments\": {},\n  \"entries\": []\n}\n".into())
}

pub async fn discover(root: &Path) -> Result<Discovery> {
    let providers = providers()?;
    let mut discovery = Discovery {
        entries: Vec::new(),
        warnings: Vec::new(),
    };
    if providers.is_empty() {
        discovery
            .warnings
            .push("未安装启动入口扩展；请安装带 run-provider.ps1 的开发扩展".into());
    }
    for provider in providers {
        let script = provider.join(PROVIDER_SCRIPT);
        let output = util::command::new_command("pwsh")
            .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-File"])
            .arg(&script)
            .arg("-Root")
            .arg(root)
            .current_dir(root)
            .kill_on_drop(true)
            .output()
            .await
            .with_context(|| format!("无法启动运行入口扩展：{}", provider.display()))?;
        ensure!(
            output.status.success(),
            "运行入口扩展失败（{}）：{}",
            provider.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
        ensure!(
            output.stdout.len() <= 8 * 1024 * 1024,
            "运行入口扩展输出过大：{}",
            provider.display()
        );
        let response: ProviderOutput = serde_json::from_slice(&output.stdout)
            .with_context(|| format!("运行入口扩展返回无效 JSON：{}", provider.display()))?;
        ensure!(
            response.version == 1,
            "运行入口扩展协议版本不支持：{}",
            provider.display()
        );
        for entry in response.entries {
            ensure!(
                !entry.id.is_empty() && !entry.task.command.is_empty(),
                "运行入口扩展返回空入口：{}",
                provider.display()
            );
            ensure!(
                !entry.file.is_absolute()
                    && !entry
                        .file
                        .components()
                        .any(|component| matches!(component, std::path::Component::ParentDir)),
                "运行入口扩展返回项目外文件：{}",
                provider.display()
            );
            if let Some(index) = discovery
                .entries
                .iter()
                .position(|existing: &RunEntry| existing.id == entry.id)
            {
                discovery.entries.remove(index);
            }
            discovery.entries.push(entry);
        }
        discovery.warnings.extend(response.warnings);
    }
    Ok(discovery)
}
