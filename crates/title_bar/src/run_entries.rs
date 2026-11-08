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

fn provider_shell() -> Option<PathBuf> {
    if cfg!(windows) {
        if let Some(path) = std::env::var_os("PATH") {
            let directories: Vec<_> = std::env::split_paths(&path).collect();
            for name in ["powershell.exe", "pwsh.exe"] {
                if let Some(candidate) = directories
                    .iter()
                    .map(|directory| directory.join(name))
                    .find(|candidate| candidate.is_file())
                {
                    return Some(candidate);
                }
            }
        }
        let shell = std::env::var_os("SystemRoot")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\Windows"))
            .join(r"System32\WindowsPowerShell\v1.0\powershell.exe");
        return shell.is_file().then_some(shell);
    }
    if let Some(path) = std::env::var_os("PATH") {
        for directory in std::env::split_paths(&path) {
            let candidate = directory.join(if cfg!(windows) { "pwsh.exe" } else { "pwsh" });
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    for path in [
        "/opt/homebrew/bin/pwsh",
        "/usr/local/bin/pwsh",
        "/usr/bin/pwsh",
    ] {
        if Path::new(path).is_file() {
            return Some(PathBuf::from(path));
        }
    }
    None
}

fn read_run_config(path: &Path) -> Result<serde_json::Value> {
    if !path.is_file() {
        return Ok(serde_json::Value::Null);
    }
    let text = fs::read_to_string(path).context("无法读取项目运行配置")?;
    serde_json::from_str(text.trim_start_matches('\u{feff}')).context("项目运行配置不是有效 JSON")
}

fn encode_arguments(value: &serde_json::Value) -> String {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|value| value.as_str())
        .map(|argument| format!("{}:{argument}", argument.len()))
        .collect()
}

fn environment_values(
    value: &serde_json::Value,
) -> Result<serde_json::Map<String, serde_json::Value>> {
    let mut environment = serde_json::Map::new();
    for (key, value) in value.as_object().into_iter().flatten() {
        let text = match value {
            serde_json::Value::String(value) => value.clone(),
            serde_json::Value::Number(_) | serde_json::Value::Bool(_) => value.to_string(),
            serde_json::Value::Null => String::new(),
            _ => anyhow::bail!("运行环境变量 {key} 必须为字符串或标量"),
        };
        environment.insert(key.clone(), text.into());
    }
    Ok(environment)
}

fn java_source_files(directory: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_symlink() {
            continue;
        }
        if path.is_dir() {
            if !matches!(
                entry.file_name().to_str(),
                Some(
                    ".git"
                        | ".zed"
                        | ".idea"
                        | "target"
                        | "node_modules"
                        | "temp"
                        | "build"
                        | "dist"
                        | ".venv"
                        | "venv"
                )
            ) {
                java_source_files(&path, files)?;
            }
        } else if path
            .extension()
            .is_some_and(|extension| extension == "java")
        {
            files.push(path);
        }
    }
    Ok(())
}

fn native_java_entries(root: &Path, provider: &Path) -> Result<Vec<RunEntry>> {
    let helper = provider.join("scripts/JavaRunner.java");
    let config = native_run_config(root)?;
    let settings = &config["environments"]["java"];
    if settings["runtime"].as_str().unwrap_or_default().is_empty() || !helper.is_file() {
        return Ok(Vec::new());
    }
    let java_home = PathBuf::from(settings["runtime"].as_str().unwrap_or_default());
    let java = java_home.join(if cfg!(windows) {
        "bin/java.exe"
    } else {
        "bin/java"
    });
    ensure!(
        java_home.is_absolute() && java.is_file(),
        "java.runtime 必须指向有效的 JDK 目录"
    );
    let mut maven = PathBuf::from(settings["build_tool"].as_str().unwrap_or_default());
    if maven.is_dir() {
        let candidates: &[&str] = if cfg!(windows) {
            &["bin/mvnd.exe", "bin/mvn.cmd", "mvnd.exe", "mvn.cmd"]
        } else {
            &["bin/mvnd", "bin/mvn", "mvnd", "mvn"]
        };
        if let Some(candidate) = candidates
            .iter()
            .map(|name| maven.join(name))
            .find(|path| path.is_file())
        {
            maven = candidate;
        }
    }
    let mut environment = serde_json::Map::new();
    let private = read_run_config(&root.join(".zed/run-config.json"))?;
    for variables in [&private["env"], &settings["env"]] {
        environment.extend(environment_values(variables)?);
    }
    environment.insert(
        "JAVA_HOME".into(),
        java_home.to_string_lossy().as_ref().into(),
    );
    environment.insert(
        "ZED_JAVA_NATIVE_ROOT".into(),
        root.to_string_lossy().as_ref().into(),
    );
    environment.insert(
        "ZED_JAVA_NATIVE_MAVEN".into(),
        maven.to_string_lossy().as_ref().into(),
    );
    environment.insert(
        "ZED_JAVA_NATIVE_VM_ARGS".into(),
        encode_arguments(&settings["jvm_args"]).into(),
    );
    environment.insert(
        "ZED_JAVA_NATIVE_ARGS".into(),
        encode_arguments(&settings["args"]).into(),
    );
    let mut files = Vec::new();
    let git_files = util::command::new_std_command("git")
        .args([
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ])
        .current_dir(root)
        .output();
    if let Ok(output) = git_files.as_ref()
        && output.status.success()
    {
        for file in output
            .stdout
            .split(|byte| *byte == 0)
            .filter(|file| file.ends_with(b".java"))
        {
            files.push(root.join(std::str::from_utf8(file).context("Java 路径不是 UTF-8")?));
        }
    } else {
        java_source_files(root, &mut files)?;
    }
    let mut entries = Vec::new();
    let scan_config = read_run_config(&root.join(".zed/runner-ignore.json"))?;
    for file in files {
        if !file.is_file() || file.is_symlink() {
            continue;
        }
        let relative_path = file.strip_prefix(root)?;
        if let Some(directories) = scan_config["scan_directories"].as_array()
            && !directories
                .iter()
                .filter_map(|directory| directory.as_str())
                .any(|directory| directory == "." || relative_path.starts_with(directory))
        {
            continue;
        }
        if scan_config["ignore_directories"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|directory| directory.as_str())
            .any(|directory| {
                relative_path.starts_with(directory)
                    || relative_path
                        .components()
                        .any(|component| component.as_os_str() == directory)
            })
        {
            continue;
        }
        let source = fs::read_to_string(&file)?;
        let normalized = source
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .replace("main (", "main(");
        if !normalized.contains("public static void main(") {
            continue;
        }
        let Some(class) = file.file_stem().and_then(|name| name.to_str()) else {
            continue;
        };
        let package = source
            .lines()
            .find_map(|line| {
                line.trim()
                    .strip_prefix("package ")
                    .and_then(|line| line.split(';').next())
            })
            .unwrap_or_default();
        let main = if package.is_empty() {
            class.to_string()
        } else {
            format!("{}.{class}", package.trim())
        };
        let module = file
            .ancestors()
            .skip(1)
            .take_while(|path| path.starts_with(root))
            .find(|path| path.join("pom.xml").is_file());
        let relative = file
            .strip_prefix(root)?
            .to_string_lossy()
            .replace('\\', "/");
        let mut environment = environment.clone();
        let Some(module) = module else {
            let arguments: Vec<_> = settings["jvm_args"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|value| value.as_str())
                .map(str::to_owned)
                .chain(std::iter::once(file.to_string_lossy().into_owned()))
                .chain(
                    settings["args"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|value| value.as_str())
                        .map(str::to_owned),
                )
                .collect();
            let entry = native_entry(
                format!("java:{relative}"),
                "java",
                &relative,
                &format!("Java · {main}"),
                &java,
                &arguments,
                environment,
                root,
            )?;
            entries.push(entry);
            continue;
        };
        if !maven.is_absolute() || !maven.is_file() {
            continue;
        }
        environment.insert(
            "ZED_JAVA_NATIVE_MODULE".into(),
            module.to_string_lossy().as_ref().into(),
        );
        environment.insert("ZED_JAVA_NATIVE_MAIN".into(), main.clone().into());
        environment.insert("ZED_JAVA_NATIVE_MODE".into(), "run".into());
        let arguments = vec![
            "-Dfile.encoding=UTF-8".into(),
            "-Dstdout.encoding=UTF-8".into(),
            "-Dstderr.encoding=UTF-8".into(),
            helper.to_string_lossy().into_owned(),
        ];
        let mut entry = native_entry(
            format!("java:{relative}"),
            "java",
            &relative,
            &format!("Java · {main}"),
            &java,
            &arguments,
            environment.clone(),
            root,
        )?;
        let mut build_task = entry.task.clone();
        build_task
            .env
            .insert("ZED_JAVA_NATIVE_MODE".into(), "build".into());
        entry.debug = Some(serde_json::from_value(serde_json::json!({
            "adapter": "Java", "label": format!("Java · {main}"), "request": "launch", "mainClass": main,
            "cwd": root, "env": environment, "encoding": "UTF-8", "javaExec": java,
            "vmArgs": settings["jvm_args"].as_array().cloned().unwrap_or_default(),
            "args": settings["args"].as_array().cloned().unwrap_or_default(), "build": build_task
        }))?);
        entries.push(entry);
        if settings["hot_reload"].as_bool() == Some(true) {
            environment.insert("ZED_JAVA_NATIVE_MODE".into(), "hot".into());
            let mut hot_entry = native_entry(
                format!("java-hot-reload:{relative}"),
                "java",
                &relative,
                &format!("Java 热更新 · {main}"),
                &java,
                &arguments,
                environment,
                root,
            )?;
            hot_entry.matching_extensions = vec!["java".into(), "xml".into(), "yaml".into()];
            entries.push(hot_entry);
        }
    }
    Ok(entries)
}

fn native_entry(
    id: String,
    language: &str,
    file: &str,
    label: &str,
    command: &Path,
    arguments: &[String],
    mut environment: serde_json::Map<String, serde_json::Value>,
    cwd: &Path,
) -> Result<RunEntry> {
    let kind = if cfg!(windows) {
        util::shell::ShellKind::Cmd
    } else {
        util::shell::ShellKind::Posix
    };
    let quote = |value: &str| {
        kind.try_quote(value)
            .map(|value| value.into_owned())
            .context("运行参数无法由系统 Shell 表达")
    };
    let shell = if cfg!(windows) {
        std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".into())
    } else {
        "/bin/sh".into()
    };
    if !cfg!(windows) && !environment.contains_key("PATH") {
        let mut directories = vec![
            command.parent().context("运行工具目录无效")?.to_path_buf(),
            PathBuf::from("/opt/homebrew/bin"),
            PathBuf::from("/usr/local/bin"),
        ];
        directories.extend(std::env::split_paths(
            &std::env::var_os("PATH").unwrap_or_else(|| "/usr/bin:/bin".into()),
        ));
        environment.insert(
            "PATH".into(),
            std::env::join_paths(directories)?
                .to_string_lossy()
                .as_ref()
                .into(),
        );
    }
    let quoted_command = if cfg!(windows) {
        let command = command.to_string_lossy();
        anyhow::ensure!(
            !command.contains(['"', '%', '\r', '\n']),
            "运行工具路径包含 CMD 无法安全表达的字符"
        );
        format!("\"{command}\"")
    } else {
        quote(&command.to_string_lossy())?
    };
    Ok(serde_json::from_value(serde_json::json!({
        "id": id, "language": language, "file": file, "matching_extensions": match language {"python" => vec!["py"], "javascript" => vec!["js", "jsx", "ts", "tsx", "json"], _ => vec![language]},
        "task": {"label": label, "command": quoted_command, "args": arguments.iter().map(|argument| quote(argument)).collect::<Result<Vec<_>>>()?,
            "cwd": cwd, "env": environment, "shell": {"program": shell}, "save": "all", "allow_concurrent_runs": false, "show_command": false, "show_summary": true}
    }))?)
}

fn tool_on_path(names: &[&str]) -> Option<PathBuf> {
    let mut directories: Vec<_> = std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect())
        .unwrap_or_default();
    if !cfg!(windows) {
        directories.extend([
            PathBuf::from("/opt/homebrew/bin"),
            PathBuf::from("/usr/local/bin"),
            PathBuf::from("/usr/bin"),
        ]);
    }
    names
        .iter()
        .flat_map(|name| {
            directories
                .iter()
                .map(move |directory| directory.join(name))
        })
        .find(|path| path.is_file())
}

fn native_template() -> Result<String> {
    let java = std::env::var_os("JAVA_HOME")
        .map(PathBuf::from)
        .filter(|path| {
            path.join(if cfg!(windows) {
                "bin/java.exe"
            } else {
                "bin/java"
            })
            .is_file()
        })
        .or_else(|| {
            if cfg!(target_os = "macos") {
                let output = util::command::new_std_command("/usr/libexec/java_home")
                    .output()
                    .ok()?;
                if !output.status.success() {
                    return None;
                }
                let path = PathBuf::from(String::from_utf8(output.stdout).ok()?.trim());
                return path.join("bin/java").is_file().then_some(path);
            }
            let executable = tool_on_path(if cfg!(windows) {
                &["java.exe"]
            } else {
                &["java"]
            })?;
            let executable = if cfg!(windows) {
                executable
            } else {
                fs::canonicalize(executable).ok()?
            };
            executable.parent()?.parent().map(PathBuf::from)
        });
    let tool = |windows: &[&str], unix: &[&str]| {
        tool_on_path(if cfg!(windows) { windows } else { unix })
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default()
    };
    Ok(serde_json::to_string_pretty(
        &serde_json::json!({"environments": {
        "java": {"runtime": java.map(|path| path.to_string_lossy().into_owned()).unwrap_or_default(), "build_tool": tool(&["mvnd.exe", "mvn.cmd"], &["mvnd", "mvn"]), "hot_reload": false, "env": {}, "args": [], "jvm_args": []},
        "python": {"runtime": tool(&["python.exe", "python3.exe"], &["python3", "python"]), "uv": tool(&["uv.exe"], &["uv"]), "env": {}},
        "go": {"runtime": tool(&["go.exe"], &["go"]), "env": {}},
        "javascript": {"runtime": tool(&["npm.cmd", "npm.exe"], &["npm"]), "env": {}}
    }, "entries": []}),
    )?)
}

fn native_run_config(root: &Path) -> Result<serde_json::Value> {
    let mut config = read_run_config(&root.join(CONFIG_PATH))?;
    let detected: serde_json::Value = serde_json::from_str(&native_template()?)?;
    if config.is_null() {
        return Ok(detected);
    }
    ensure!(config.is_object(), "运行环境配置必须是 JSON 对象");
    ensure!(
        config["environments"].is_null() || config["environments"].is_object(),
        "environments 必须是 JSON 对象"
    );
    for (language, keys) in [
        ("java", &["runtime", "build_tool"][..]),
        ("python", &["runtime", "uv"][..]),
        ("go", &["runtime"][..]),
        ("javascript", &["runtime"][..]),
    ] {
        ensure!(
            config["environments"][language].is_null()
                || config["environments"][language].is_object(),
            "{language} 运行环境必须是 JSON 对象"
        );
        for key in keys {
            let configured = &config["environments"][language][key];
            let valid = configured.as_str().is_some_and(|value| {
                let path = Path::new(value);
                path.is_absolute()
                    && if language == "java" && *key == "runtime" {
                        path.join(if cfg!(windows) {
                            "bin/java.exe"
                        } else {
                            "bin/java"
                        })
                        .is_file()
                    } else if language == "java" && *key == "build_tool" {
                        path.is_file() || path.is_dir()
                    } else {
                        path.is_file()
                    }
            });
            if !valid {
                config["environments"][language][key] =
                    detected["environments"][language][key].clone();
            }
        }
    }
    Ok(config)
}

fn native_runtime(settings: &serde_json::Value, key: &str) -> Result<Option<PathBuf>> {
    let Some(value) = settings[key].as_str().filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    let path = PathBuf::from(value);
    ensure!(
        path.is_absolute() && path.is_file(),
        "运行环境 {key} 必须是当前平台的可执行文件绝对路径"
    );
    Ok(Some(path))
}

fn collect_source_files(directory: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        if entry.file_type()?.is_symlink() {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            if !matches!(
                entry.file_name().to_str(),
                Some(
                    ".git"
                        | ".zed"
                        | ".idea"
                        | "target"
                        | "node_modules"
                        | "temp"
                        | "build"
                        | "dist"
                        | ".venv"
                        | "venv"
                )
            ) {
                collect_source_files(&path, files)?;
            }
        } else {
            files.push(path);
        }
    }
    Ok(())
}

fn native_other_entries(root: &Path, provider: &Path) -> Result<Vec<RunEntry>> {
    let config = native_run_config(root)?;
    let scan = read_run_config(&root.join(".zed/runner-ignore.json"))?;
    let python_settings = &config["environments"]["python"];
    let python = native_runtime(python_settings, "runtime")?;
    let uv = native_runtime(python_settings, "uv")?;
    let go = native_runtime(&config["environments"]["go"], "runtime")?;
    let npm = native_runtime(&config["environments"]["javascript"], "runtime")?;
    let mut files = Vec::new();
    let output = util::command::new_std_command("git")
        .args([
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ])
        .current_dir(root)
        .output();
    if let Ok(output) = output
        && output.status.success()
    {
        for file in output
            .stdout
            .split(|byte| *byte == 0)
            .filter(|file| !file.is_empty())
        {
            files.push(root.join(std::str::from_utf8(file)?));
        }
    } else {
        collect_source_files(root, &mut files)?;
    }
    let environment = |language: &str| environment_values(&config["environments"][language]["env"]);
    let mut entries = Vec::new();
    for file in files {
        if !file.is_file() || file.is_symlink() {
            continue;
        }
        let relative = file.strip_prefix(root)?;
        if let Some(directories) = scan["scan_directories"].as_array()
            && !directories
                .iter()
                .filter_map(|value| value.as_str())
                .any(|directory| directory == "." || relative.starts_with(directory))
        {
            continue;
        }
        if scan["ignore_directories"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|value| value.as_str())
            .any(|directory| {
                relative.starts_with(directory)
                    || relative
                        .components()
                        .any(|component| component.as_os_str() == directory)
            })
        {
            continue;
        }
        let relative = relative.to_string_lossy().replace('\\', "/");
        let Some(directory) = file.parent() else {
            continue;
        };
        match file.extension().and_then(|value| value.to_str()) {
            Some("py") if python.is_some() || uv.is_some() => {
                let source = fs::read_to_string(&file)?;
                if !source.contains("__name__")
                    || !source.contains("__main__")
                    || file
                        .file_name()
                        .is_some_and(|name| name.to_string_lossy().starts_with("test_"))
                {
                    continue;
                }
                let project = directory
                    .ancestors()
                    .take_while(|path| path.starts_with(root))
                    .find(|path| {
                        path.join("pyproject.toml").is_file()
                            || path.join("requirements.txt").is_file()
                    });
                let mut env = environment("python")?;
                if let Some(uv) = uv.as_ref() {
                    env.insert("ZED_PYTHON_UV".into(), uv.to_string_lossy().as_ref().into());
                }
                let label = format!("Python · {relative}");
                let (command, arguments, interpreter, prepare) = if let Some(project) = project {
                    let local_python = project.join(if cfg!(windows) {
                        ".venv/Scripts/python.exe"
                    } else {
                        ".venv/bin/python"
                    });
                    if project.join("pyproject.toml").is_file() && uv.is_none() {
                        continue;
                    }
                    if project.join("requirements.txt").is_file() && python.is_none() {
                        continue;
                    }
                    if project.join("pyproject.toml").is_file() && python.is_none() {
                        let Some(uv) = uv.as_ref() else { continue };
                        let prepare = native_entry(
                            "prepare".into(),
                            "python",
                            &relative,
                            "准备 Python 环境",
                            uv,
                            &[
                                "sync".into(),
                                "--no-default-groups".into(),
                                "--inexact".into(),
                                "--project".into(),
                                project.to_string_lossy().into_owned(),
                            ],
                            env.clone(),
                            root,
                        )?
                        .task;
                        (
                            uv.clone(),
                            vec![
                                "run".into(),
                                "--no-default-groups".into(),
                                "--inexact".into(),
                                "--project".into(),
                                project.to_string_lossy().into_owned(),
                                file.to_string_lossy().into_owned(),
                            ],
                            local_python,
                            Some(prepare),
                        )
                    } else {
                        let Some(python) = python.as_ref() else {
                            continue;
                        };
                        let helper = provider.join("scripts/PythonRunner.py");
                        ensure!(helper.is_file(), "请更新 zed-runner 的 Python 运行器");
                        let prefix = vec![
                            helper.to_string_lossy().into_owned(),
                            project.to_string_lossy().into_owned(),
                        ];
                        let prepare = native_entry(
                            "prepare".into(),
                            "python",
                            &relative,
                            "准备 Python 环境",
                            python,
                            &[prefix.clone(), vec!["--prepare".into()]].concat(),
                            env.clone(),
                            root,
                        )?
                        .task;
                        (
                            python.clone(),
                            [prefix, vec![file.to_string_lossy().into_owned()]].concat(),
                            local_python,
                            Some(prepare),
                        )
                    }
                } else {
                    let local = directory
                        .ancestors()
                        .take_while(|path| path.starts_with(root))
                        .flat_map(|path| {
                            [
                                path.join(if cfg!(windows) {
                                    ".venv/Scripts/python.exe"
                                } else {
                                    ".venv/bin/python"
                                }),
                                path.join(if cfg!(windows) {
                                    "venv/Scripts/python.exe"
                                } else {
                                    "venv/bin/python"
                                }),
                            ]
                        })
                        .find(|path| path.is_file());
                    let Some(interpreter) = local.or_else(|| python.clone()) else {
                        continue;
                    };
                    (
                        interpreter.clone(),
                        vec![file.to_string_lossy().into_owned()],
                        interpreter,
                        None,
                    )
                };
                let mut entry = native_entry(
                    format!("python:{relative}"),
                    "python",
                    &relative,
                    &label,
                    &command,
                    &arguments,
                    env.clone(),
                    root,
                )?;
                let mut debug = serde_json::json!({"adapter": "Debugpy", "label": label, "request": "launch", "program": file, "cwd": root, "env": env, "python": interpreter});
                if let Some(prepare) = prepare {
                    debug["build"] = serde_json::to_value(prepare)?;
                }
                entry.debug = Some(serde_json::from_value(debug)?);
                entries.push(entry);
            }
            Some("go") if go.is_some() => {
                let source = fs::read_to_string(&file)?;
                if !source
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
                    .replace("main (", "main(")
                    .contains("func main(")
                {
                    continue;
                }
                if !directory
                    .ancestors()
                    .take_while(|path| path.starts_with(root))
                    .any(|path| path.join("go.mod").is_file())
                {
                    continue;
                }
                let Some(go) = go.as_ref() else { continue };
                let env = environment("go")?;
                let mut entry = native_entry(
                    format!("go:{relative}"),
                    "go",
                    &relative,
                    &format!("Go · {relative}"),
                    go,
                    &["run".into(), ".".into()],
                    env.clone(),
                    directory,
                )?;
                entry.debug = Some(serde_json::from_value(
                    serde_json::json!({"adapter": "Delve", "label": entry.task.label, "request": "launch", "mode": "debug", "program": directory, "cwd": directory, "env": env}),
                )?);
                entries.push(entry);
            }
            Some("json")
                if file.file_name().is_some_and(|name| name == "package.json") && npm.is_some() =>
            {
                let package = read_run_config(&file)?;
                let Some(npm) = npm.as_ref() else { continue };
                for name in package["scripts"]
                    .as_object()
                    .into_iter()
                    .flat_map(|scripts| scripts.keys())
                {
                    entries.push(native_entry(
                        format!("javascript:{relative}:{name}"),
                        "javascript",
                        &relative,
                        &format!("npm · {name} · {relative}"),
                        npm,
                        &["run".into(), name.clone()],
                        environment("javascript")?,
                        directory,
                    )?);
                }
            }
            _ => {}
        }
    }
    for entry in config["entries"].as_array().into_iter().flatten() {
        let entry: RunEntry = serde_json::from_value(entry.clone())?;
        ensure!(
            !entry.id.is_empty() && !entry.task.command.is_empty(),
            "自定义运行入口缺少名称或命令"
        );
        ensure!(
            !entry.file.is_absolute()
                && !entry
                    .file
                    .components()
                    .any(|component| matches!(component, std::path::Component::ParentDir)),
            "自定义运行入口必须位于项目内"
        );
        entries.push(entry);
    }
    Ok(entries)
}

pub fn config_template() -> Result<String> {
    for provider in providers()? {
        if provider.join("scripts/JavaRunner.java").is_file() {
            return native_template();
        }
        let generator = provider.join(TEMPLATE_SCRIPT);
        if generator.is_file()
            && let Some(shell) = provider_shell()
        {
            let output = std::process::Command::new(shell)
                .args([
                    "-NoLogo",
                    "-NoProfile",
                    "-NonInteractive",
                    "-ExecutionPolicy",
                    "Bypass",
                    "-File",
                ])
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
        if provider.join("scripts/JavaRunner.java").is_file() {
            let config = native_run_config(root)?;
            for (language, marker, message) in [
                (
                    "java",
                    "pom.xml",
                    "未检测到 JDK；请安装 JDK 或配置 JAVA_HOME",
                ),
                ("go", "go.mod", "未检测到 Go；请安装 Go 并加入 PATH"),
                (
                    "python",
                    "pyproject.toml",
                    "未检测到 Python；请安装 Python 或选择项目虚拟环境",
                ),
                (
                    "javascript",
                    "package.json",
                    "未检测到 npm；请安装 Node.js 并加入 PATH",
                ),
            ] {
                if root.join(marker).is_file()
                    && config["environments"][language]["runtime"]
                        .as_str()
                        .unwrap_or_default()
                        .is_empty()
                {
                    discovery.warnings.push(message.into());
                }
            }
            discovery
                .entries
                .extend(native_java_entries(root, &provider)?);
            discovery
                .entries
                .extend(native_other_entries(root, &provider)?);
            continue;
        }
        let Some(shell) = provider_shell() else {
            continue;
        };
        let script = provider.join(PROVIDER_SCRIPT);
        let output = util::command::new_command(shell)
            .args([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
            ])
            .arg(&script)
            .arg("-Root")
            .arg(root)
            .current_dir(root)
            .kill_on_drop(true)
            .output()
            .await
            .with_context(|| {
                format!(
                    "无法启动运行入口扩展（Windows 使用系统 PowerShell，其他平台需要 pwsh）：{}",
                    provider.display()
                )
            })?;
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

#[cfg(test)]
mod native_tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn native_command_with_spaces_runs_in_cmd() -> Result<()> {
        use std::os::windows::process::CommandExt;
        let root = std::env::temp_dir().join(format!("zed cmd regression {}", std::process::id()));
        fs::create_dir_all(&root)?;
        let command = root.join("npm fixture.cmd");
        fs::write(
            &command,
            "@echo off\r\necho NPM_ARGUMENT=%~1\r\nexit /b 17\r\n",
        )?;
        let entry = native_entry(
            "test".into(),
            "javascript",
            "package.json",
            "test",
            &command,
            &["argument with spaces".into()],
            serde_json::Map::new(),
            &root,
        )?;
        let (program, arguments) = task::ShellBuilder::new(&entry.task.shell, true)
            .non_interactive()
            .build_no_quote(Some(entry.task.command), &entry.task.args);
        let mut process = std::process::Command::new(program);
        for argument in arguments {
            process.raw_arg(argument);
        }
        let output = process.output()?;
        assert_eq!(
            output.status.code(),
            Some(17),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("NPM_ARGUMENT=argument with spaces")
        );
        fs::remove_file(&command)?;
        fs::remove_dir(&root)?;
        Ok(())
    }

    fn write(path: &Path, value: &str) -> Result<()> {
        fs::create_dir_all(path.parent().context("fixture directory")?)?;
        fs::write(path, value)?;
        Ok(())
    }

    #[test]
    fn native_entries_do_not_require_powershell() -> Result<()> {
        let temporary = std::env::temp_dir();
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos();
        let fixture = temporary.join(format!("zed-native-中文-'{}-{unique}", std::process::id()));
        let root = fixture.join("project");
        let provider = fixture.join("provider");
        let jdk = fixture.join("jdk");
        write(
            &jdk.join(if cfg!(windows) {
                "bin/java.exe"
            } else {
                "bin/java"
            }),
            "fixture",
        )?;
        let executable = fixture.join(if cfg!(windows) { "tool.exe" } else { "tool" });
        write(&executable, "fixture")?;
        write(&provider.join("scripts/JavaRunner.java"), "fixture")?;
        write(&provider.join("scripts/PythonRunner.py"), "fixture")?;
        write(&root.join("pom.xml"), "<project />")?;
        write(
            &root.join("src/main/java/Main.java"),
            "package sample;\npublic class Main { public static void main(String[] args) {} }",
        )?;
        write(&root.join("job.py"), "if __name__ == '__main__': pass")?;
        write(&root.join("requirements.txt"), "")?;
        write(&root.join("go.mod"), "module example")?;
        write(&root.join("cmd/main.go"), "package main\nfunc main() {}")?;
        write(
            &root.join("package.json"),
            r#"{"scripts":{"start":"node app.js"}}"#,
        )?;
        write(
            &root.join(CONFIG_PATH),
            &serde_json::to_string(&serde_json::json!({"environments": {
            "java": {"runtime": jdk, "build_tool": executable, "hot_reload": true, "args": ["中文", "quoted argument"], "env": {}},
            "python": {"runtime": executable, "env": {}}, "go": {"runtime": executable, "env": {}}, "javascript": {"runtime": executable, "env": {}}
        }, "entries": []}))?,
        )?;
        let mut entries = native_java_entries(&root, &provider)?;
        entries.extend(native_other_entries(&root, &provider)?);
        for id in [
            "java:src/main/java/Main.java",
            "java-hot-reload:src/main/java/Main.java",
            "python:job.py",
            "go:cmd/main.go",
            "javascript:package.json:start",
        ] {
            assert!(entries.iter().any(|entry| entry.id == id), "missing {id}");
        }
        for entry in &entries {
            let runtime = serde_json::to_string(&entry.task)?;
            assert!(
                !runtime.contains("powershell")
                    && !runtime.contains("pwsh")
                    && !runtime.contains(".ps1")
            );
        }
        let hot = entries
            .iter()
            .find(|entry| entry.id.starts_with("java-hot-reload:"))
            .context("hot entry")?;
        assert!(
            hot.matching_extensions
                .iter()
                .any(|extension| extension == "xml")
        );
        assert_eq!(
            hot.task.env.get("ZED_JAVA_NATIVE_MODE").map(String::as_str),
            Some("hot")
        );
        assert!(
            hot.task
                .env
                .get("ZED_JAVA_NATIVE_ARGS")
                .is_some_and(|value| value.starts_with("6:中文"))
        );
        write(
            &root.join(".zed/runner-ignore.json"),
            r#"{"scan_directories":["src"],"ignore_directories":[]}"#,
        )?;
        assert!(native_other_entries(&root, &provider)?.is_empty());
        assert_eq!(native_java_entries(&root, &provider)?.len(), 2);
        ensure!(
            fixture.starts_with(&temporary),
            "fixture cleanup must stay in temporary directory"
        );
        fs::remove_dir_all(fixture)?;
        Ok(())
    }

    #[test]
    fn automatic_runtime_detection_handles_moved_projects() -> Result<()> {
        let temporary = std::env::temp_dir();
        let root_path = temporary.join(format!(
            "zed-runtime-detection-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        ));
        let root = root_path.as_path();
        let detected: serde_json::Value = serde_json::from_str(&native_template()?)?;
        assert_eq!(native_run_config(root)?, detected);
        write(
            &root.join(CONFIG_PATH),
            r#"{"environments":{"java":{"runtime":"C:/missing-jdk","env":{"KEEP":"yes"}},"python":{"runtime":"/missing/python"}},"entries":[{"id":"custom"}]}"#,
        )?;
        let config = native_run_config(root)?;
        assert_eq!(
            config["environments"]["java"]["runtime"],
            detected["environments"]["java"]["runtime"]
        );
        assert_eq!(
            config["environments"]["python"]["runtime"],
            detected["environments"]["python"]["runtime"]
        );
        assert_eq!(config["environments"]["java"]["env"]["KEEP"], "yes");
        assert_eq!(config["entries"][0]["id"], "custom");
        let executable = root.join("custom-python");
        write(&executable, "fixture")?;
        let custom = serde_json::json!({"environments":{"python":{"runtime":executable}}});
        write(&root.join(CONFIG_PATH), &serde_json::to_string(&custom)?)?;
        assert_eq!(
            native_run_config(root)?["environments"]["python"]["runtime"],
            custom["environments"]["python"]["runtime"]
        );
        for invalid in [
            "[]",
            r#"{"environments":false}"#,
            r#"{"environments":{"java":false}}"#,
        ] {
            write(&root.join(CONFIG_PATH), invalid)?;
            assert!(native_run_config(root).is_err());
        }
        ensure!(
            root.starts_with(&temporary),
            "fixture must stay in temporary directory"
        );
        fs::remove_dir_all(root)?;
        Ok(())
    }
}
