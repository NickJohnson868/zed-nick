use anyhow::{Context as _, Result};
use base64::Engine as _;
use regex::Regex;
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
use task::{SaveStrategy, Shell, TaskTemplate};

pub const CONFIG_PATH: &str = ".zed/run-environments.json";
pub const RUN_TAG: &str = "toolbar-run";
pub const CONFIG_TEMPLATE: &str = r#"{
  "environments": {
    "java": { "env": {}, "args": [], "jvm_args": [] },
    "python": { "env": {}, "args": [] },
    "javascript": { "runtime": "npm", "env": {} },
    "rust": { "runtime": "cargo", "env": {} },
    "go": { "runtime": "go", "env": {} },
    "csharp": { "runtime": "dotnet", "env": {} }
  },
  "entries": []
}
"#;

#[derive(Clone)]
pub struct RunEntry {
    pub id: String,
    pub language: String,
    pub file: PathBuf,
    pub task: TaskTemplate,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunEnvironments {
    #[serde(default)]
    environments: BTreeMap<String, Environment>,
    #[serde(default)]
    entries: Vec<ConfiguredEntry>,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Environment {
    runtime: Option<String>,
    build_tool: Option<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    jvm_args: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfiguredEntry {
    id: String,
    language: String,
    file: PathBuf,
    task: TaskTemplate,
}

pub struct Discovery {
    pub entries: Vec<RunEntry>,
    pub warnings: Vec<String>,
}

pub fn discover(root: &Path, files: Vec<PathBuf>) -> Result<Discovery> {
    let mut config = match std::fs::read_to_string(root.join(CONFIG_PATH)) {
        Ok(text) => serde_json::from_str::<RunEnvironments>(&text).context("运行环境配置无效")?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => RunEnvironments::default(),
        Err(error) => return Err(error).context("无法读取运行环境配置"),
    };
    if config
        .environments
        .get("java")
        .and_then(|environment| environment.runtime.as_ref())
        .is_none()
    {
        if let Ok(text) = std::fs::read_to_string(root.join(".zed/run-config.json")) {
            if let Ok(legacy) = serde_json::from_str::<serde_json::Value>(&text) {
                if let Some(jdk) = legacy.get("jdkPath").and_then(|jdk| jdk.as_str()) {
                    config
                        .environments
                        .entry("java".into())
                        .or_default()
                        .runtime = Some(jdk.into());
                }
            }
        }
    }
    let java_main = Regex::new(
        r"\b(?:public\s+static|static\s+public)\s+void\s+main\s*\(\s*(?:java\.lang\.)?String\s*(?:\[\s*\]|\.\.\.)",
    )?;
    let java_package = Regex::new(r"(?m)^\s*package\s+([\w.]+)\s*;")?;
    let python_main = Regex::new(r#"(?m)^\s*if\s+__name__\s*==\s*["']__main__["']\s*:"#)?;
    let go_main = Regex::new(r"(?m)^func\s+main\s*\(\s*\)")?;
    let mut result = Discovery {
        entries: Vec::new(),
        warnings: Vec::new(),
    };
    let mut spring_entries = Vec::new();
    for file in files {
        let name = file
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        let extension = file
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or_default();
        if !matches!(extension, "java" | "py" | "go" | "csproj")
            && !matches!(name, "package.json" | "Cargo.toml")
        {
            continue;
        }
        let path = root.join(&file);
        let text = match std::fs::metadata(&path).and_then(|metadata| {
            if metadata.len() > 1024 * 1024 {
                return Ok(String::new());
            }
            std::fs::read_to_string(&path)
        }) {
            Ok(text) => text,
            Err(error) => {
                result.warnings.push(format!("{}: {error}", file.display()));
                continue;
            }
        };
        let parent = file.parent().unwrap_or(Path::new(""));
        match extension {
            "java" if java_main.is_match(&without_comments(&text)) => {
                if text.contains("@SpringBootApplication") {
                    spring_entries.push(file.clone());
                }
                let class = file
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .unwrap_or_default();
                let class = java_package
                    .captures(&text)
                    .and_then(|capture| capture.get(1))
                    .map(|package| format!("{}.{class}", package.as_str()))
                    .unwrap_or_else(|| class.into());
                let module = nearest_manifest(root, parent, "pom.xml");
                let environment = config.environments.get("java");
                let runtime = runtime(environment, "java");
                let data = serde_json::json!({
                    "root": root, "file": path, "class": class, "module": module,
                    "java": runtime, "maven": environment.and_then(|environment| environment.build_tool.as_deref()).unwrap_or(""),
                    "args": environment.map(|environment| environment.args.clone()).unwrap_or_default(),
                    "jvmArgs": environment.map(|environment| environment.jvm_args.clone()).unwrap_or_default(),
                    "envKeys": environment.map(|environment| environment.env.keys().cloned().collect::<Vec<_>>()).unwrap_or_default()
                });
                let encoded = base64::engine::general_purpose::STANDARD.encode(
                    include_str!("run_java.ps1")
                        .encode_utf16()
                        .flat_map(u16::to_le_bytes)
                        .collect::<Vec<_>>(),
                );
                let mut task = command_task(
                    root,
                    &config,
                    "java",
                    &format!("Java · {class}"),
                    "pwsh",
                    vec![
                        "-NoLogo".into(),
                        "-NoProfile".into(),
                        "-NonInteractive".into(),
                        "-EncodedCommand".into(),
                        encoded,
                    ],
                );
                task.env.insert("ZED_JAVA_ENTRY".into(), data.to_string());
                result.entries.push(RunEntry {
                    id: file.to_string_lossy().into_owned(),
                    language: "java".into(),
                    file,
                    task,
                });
            }
            "py" if python_main.is_match(&text)
                || matches!(name, "main.py" | "__main__.py" | "app.py" | "manage.py") =>
            {
                let environment = config.environments.get("python");
                let interpreter = environment
                    .and_then(|environment| environment.runtime.clone())
                    .unwrap_or_else(|| {
                        [
                            root.join(".venv/Scripts/python.exe"),
                            root.join(".venv/bin/python"),
                            root.join("venv/Scripts/python.exe"),
                        ]
                        .into_iter()
                        .find(|path| path.is_file())
                        .map(|path| path.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "python".into())
                    });
                let mut args = vec!["-u".into(), path.to_string_lossy().into_owned()];
                args.extend(
                    environment
                        .map(|environment| environment.args.clone())
                        .unwrap_or_default(),
                );
                let task = command_task(
                    root,
                    &config,
                    "python",
                    &format!("Python · {}", file.display()),
                    &interpreter,
                    args,
                );
                result.entries.push(RunEntry {
                    id: file.to_string_lossy().into_owned(),
                    language: "python".into(),
                    file,
                    task,
                });
            }
            "go" if text.contains("package main") && go_main.is_match(&text) => {
                let command = runtime(config.environments.get("go"), "go");
                let task = command_task(
                    &root.join(parent),
                    &config,
                    "go",
                    &format!("Go · {}", parent.display()),
                    &command,
                    vec!["run".into(), ".".into()],
                );
                result.entries.push(RunEntry {
                    id: format!("go:{}", parent.display()),
                    language: "go".into(),
                    file,
                    task,
                });
            }
            "csproj"
                if text.contains("<OutputType>Exe</OutputType>")
                    || text.contains("Microsoft.NET.Sdk.Web") =>
            {
                let command = runtime(config.environments.get("csharp"), "dotnet");
                let task = command_task(
                    root,
                    &config,
                    "csharp",
                    &format!(".NET · {}", file.display()),
                    &command,
                    vec![
                        "run".into(),
                        "--project".into(),
                        path.to_string_lossy().into_owned(),
                    ],
                );
                result.entries.push(RunEntry {
                    id: file.to_string_lossy().into_owned(),
                    language: "csharp".into(),
                    file,
                    task,
                });
            }
            _ if name == "package.json" => match serde_json::from_str::<serde_json::Value>(&text) {
                Ok(package) => {
                    if let Some(scripts) = package
                        .get("scripts")
                        .and_then(|scripts| scripts.as_object())
                    {
                        for script in scripts.keys() {
                            let command = runtime(config.environments.get("javascript"), "npm");
                            let task = command_task(
                                &root.join(parent),
                                &config,
                                "javascript",
                                &format!("npm · {} · {script}", parent.display()),
                                &command,
                                vec!["run".into(), script.clone()],
                            );
                            result.entries.push(RunEntry {
                                id: format!("{}:{script}", file.display()),
                                language: "javascript".into(),
                                file: file.clone(),
                                task,
                            });
                        }
                    }
                }
                Err(error) => result.warnings.push(format!("{}: {error}", file.display())),
            },
            _ if name == "Cargo.toml" => {
                // Cargo remains the authority for package metadata, including custom bin paths.
                // Discovery never executes project build scripts or package-manager commands.
                if root.join(parent).join("src/main.rs").is_file() {
                    let command = runtime(config.environments.get("rust"), "cargo");
                    let task = command_task(
                        &root.join(parent),
                        &config,
                        "rust",
                        &format!("Rust · {}", parent.display()),
                        &command,
                        vec![
                            "run".into(),
                            "--manifest-path".into(),
                            path.to_string_lossy().into_owned(),
                        ],
                    );
                    result.entries.push(RunEntry {
                        id: file.to_string_lossy().into_owned(),
                        language: "rust".into(),
                        file,
                        task,
                    });
                }
            }
            _ => {}
        }
    }
    match std::fs::read_to_string(root.join(".zed/tasks.json")) {
        Ok(text) => match serde_json::from_str::<Vec<TaskTemplate>>(&text) {
            Ok(tasks) => {
                if let Some(task) = tasks
                    .into_iter()
                    .find(|task| task.tags.iter().any(|tag| tag == "primary-run"))
                    .filter(|_| !result.entries.iter().any(|entry| entry.language == "java"))
                {
                    let file = if spring_entries.len() == 1 {
                        spring_entries.first().cloned().unwrap_or_default()
                    } else {
                        PathBuf::new()
                    };
                    if !file.as_os_str().is_empty() {
                        result.entries.retain(|entry| entry.file != file);
                    }
                    result.entries.push(RunEntry {
                        id: "configured:primary".into(),
                        language: "java".into(),
                        file,
                        task,
                    });
                }
            }
            Err(error) => result.warnings.push(format!(".zed/tasks.json: {error}")),
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => result.warnings.push(format!(".zed/tasks.json: {error}")),
    }
    for entry in config.entries {
        let environment = config.environments.get(&entry.language);
        let mut task = entry.task;
        if let Some(environment) = environment {
            for (key, value) in &environment.env {
                task.env.entry(key.clone()).or_insert(value.clone());
            }
        }
        task.cwd
            .get_or_insert_with(|| root.to_string_lossy().into_owned());
        result.entries.retain(|candidate| candidate.id != entry.id);
        result.entries.push(RunEntry {
            id: entry.id,
            language: entry.language,
            file: entry.file,
            task,
        });
    }
    let primary_file = (spring_entries.len() == 1)
        .then(|| spring_entries.first())
        .flatten();
    result.entries.sort_by(|left, right| {
        (Some(&left.file) != primary_file)
            .cmp(&(Some(&right.file) != primary_file))
            .then_with(|| {
                (left.id != "configured:primary").cmp(&(right.id != "configured:primary"))
            })
            .then_with(|| left.task.label.cmp(&right.task.label))
    });
    result.entries.dedup_by(|left, right| left.id == right.id);
    Ok(result)
}

fn runtime(environment: Option<&Environment>, fallback: &str) -> String {
    environment
        .and_then(|environment| environment.runtime.clone())
        .unwrap_or_else(|| fallback.into())
}

fn command_task(
    root: &Path,
    config: &RunEnvironments,
    language: &str,
    label: &str,
    program: &str,
    args: Vec<String>,
) -> TaskTemplate {
    let quote = |value: &str| format!("'{}'", value.replace('\'', "''"));
    TaskTemplate {
        label: label.into(),
        command: format!("& {}", quote(program)),
        args: args.iter().map(|arg| quote(arg)).collect(),
        cwd: Some(root.to_string_lossy().into_owned()),
        env: config
            .environments
            .get(language)
            .map(|environment| environment.env.clone().into_iter().collect())
            .unwrap_or_default(),
        shell: Shell::WithArguments {
            program: "pwsh".into(),
            args: vec!["-NoLogo".into(), "-NoProfile".into()],
            title_override: None,
        },
        save: SaveStrategy::All,
        show_command: false,
        show_summary: true,
        ..Default::default()
    }
}

fn nearest_manifest(root: &Path, parent: &Path, name: &str) -> Option<PathBuf> {
    parent
        .ancestors()
        .map(|directory| root.join(directory).join(name))
        .find(|path| path.is_file())
}

fn without_comments(text: &str) -> String {
    // Preserve line breaks so a commented main method cannot become a runnable entry.
    let mut result = String::with_capacity(text.len());
    let mut characters = text.chars().peekable();
    let mut quote = None;
    while let Some(character) = characters.next() {
        if let Some(delimiter) = quote {
            result.push(' ');
            if character == '\\' {
                characters.next();
            } else if character == delimiter {
                quote = None;
            }
        } else if character == '"' || character == '\'' {
            quote = Some(character);
            result.push(' ');
        } else if character == '/' && characters.peek() == Some(&'/') {
            for character in characters.by_ref() {
                if character == '\n' {
                    result.push('\n');
                    break;
                }
            }
        } else if character == '/' && characters.peek() == Some(&'*') {
            characters.next();
            while let Some(character) = characters.next() {
                if character == '*' && characters.peek() == Some(&'/') {
                    characters.next();
                    break;
                }
                if character == '\n' {
                    result.push('\n');
                }
            }
            result.push(' ');
        } else {
            result.push(character);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ignores_commented_and_quoted_java_entry_points() {
        let input = "// public static void main(String[] args) {}\n/* main */\nclass Main { String s = \"public static void main(String[] args)\"; public static void main(String... args) {} }";
        let stripped = without_comments(input);
        assert_eq!(stripped.matches("public static void main").count(), 1);
        assert!(stripped.contains("main(String... args)"));
    }
}
