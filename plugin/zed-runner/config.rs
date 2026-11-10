use serde_json::{Value, json};
use std::{
    collections::hash_map::DefaultHasher,
    fs::{self, OpenOptions},
    hash::{Hash, Hasher},
    io::{self, Write},
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

pub const CONFIG_PATH: &str = ".zed/zed-runner.json";

fn read(path: &Path) -> io::Result<Value> {
    if !path.exists() {
        return Ok(Value::Null);
    }
    if fs::symlink_metadata(path)?.file_type().is_symlink() {
        return Err(io::Error::other("运行配置不能是符号链接"));
    }
    let text = fs::read_to_string(path)?;
    serde_json::from_str(text.trim_start_matches('\u{feff}'))
        .map_err(|_| io::Error::other("运行配置不是有效 JSON（未输出配置内容）"))
}

fn merge(target: &mut Value, source: Value) {
    match (target, source) {
        (Value::Object(target), Value::Object(source)) => {
            for (key, value) in source {
                merge(target.entry(key).or_insert(Value::Null), value);
            }
        }
        (target, source) => *target = source,
    }
}

fn cache_directory(root: &Path) -> PathBuf {
    let base = if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|home| PathBuf::from(home).join("Library/Caches"))
    } else {
        std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))
    }
    .unwrap_or_else(std::env::temp_dir);
    let mut hash = DefaultHasher::new();
    root.hash(&mut hash);
    base.join("zed-nick/runner")
        .join(format!("{:016x}", hash.finish()))
}

fn protect_migrated_settings(root: &Path) -> io::Result<()> {
    let output = match Command::new("git")
        .args(["rev-parse", "--git-path", "info/exclude"])
        .current_dir(root)
        .output()
    {
        Ok(output) if output.status.success() => output,
        Ok(_) => return Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    let tracked = Command::new("git")
        .args(["ls-files", "--error-unmatch", "--", CONFIG_PATH])
        .current_dir(root)
        .output()?;
    if tracked.status.success() {
        return Err(io::Error::other(
            "zed-runner.json 已被 Git 跟踪；请先取消跟踪，再迁移私密运行配置",
        ));
    }
    let path = std::str::from_utf8(&output.stdout)
        .map_err(io::Error::other)?
        .trim();
    if path.is_empty() {
        return Err(io::Error::other("Git 本地排除文件路径为空"));
    }
    let path = PathBuf::from(path);
    let path = if path.is_absolute() {
        path
    } else {
        root.join(path)
    };
    if path.is_symlink() {
        return Err(io::Error::other("不修改符号链接形式的 Git 本地排除文件"));
    }
    let existing = match fs::read(&path) {
        Ok(existing) => existing,
        Err(error) if error.kind() == io::ErrorKind::NotFound => Vec::new(),
        Err(error) => return Err(error),
    };
    let pattern = b"**/.zed/zed-runner.json";
    if existing
        .split(|byte| *byte == b'\n')
        .any(|line| line == pattern || line.strip_suffix(b"\r") == Some(pattern.as_slice()))
    {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new().append(true).create(true).open(path)?;
    if !existing.is_empty() && !existing.ends_with(b"\n") {
        file.write_all(b"\n")?;
    }
    file.write_all(pattern)?;
    file.write_all(b"\n")?;
    file.sync_all()
}

/// Keep recovery copies outside the project before retiring the split configuration.
pub fn load(root: &Path) -> io::Result<Value> {
    load_with_cache(root, &cache_directory(root))
}

fn load_with_cache(root: &Path, cache: &Path) -> io::Result<Value> {
    let destination = root.join(CONFIG_PATH);
    if destination.exists() {
        let config = read(&destination)?;
        if !config.is_object() {
            return Err(io::Error::other("zed-runner.json 必须是 JSON 对象"));
        }
        return Ok(config);
    }
    let directory = root.join(".zed");
    if directory.is_symlink() {
        return Err(io::Error::other("不能自动迁移符号链接形式的 .zed 目录"));
    }
    let sources = [
        "run-config.json",
        "run-environments.json",
        "runner-ignore.json",
    ];
    let mut config = json!({});
    let mut retired = Vec::new();
    for name in sources {
        let path = directory.join(name);
        let value = read(&path)?;
        if !value.is_null() {
            if !value.is_object() {
                return Err(io::Error::other("旧运行配置必须是 JSON 对象"));
            }
            merge(&mut config, value);
            retired.push(path);
        }
    }
    if retired.is_empty() {
        return Ok(Value::Null);
    }
    for value in [
        &config["environments"],
        &config["environments"]["java"],
        &config["env"],
        &config["environments"]["java"]["env"],
    ] {
        if !value.is_null() && !value.is_object() {
            return Err(io::Error::other(
                "运行环境和 env 必须是 JSON 对象，旧配置未迁移",
            ));
        }
    }
    // Zed tasks owned by the retired launcher are replaced by native toolbar
    // discovery. Preserve their definitions in the recovery copy, never execute them.
    if let Some(java_home) = config
        .as_object_mut()
        .and_then(|config| config.remove("jdkPath"))
    {
        if config["environments"]["java"]["runtime"].is_null() {
            config["environments"]["java"]["runtime"] = java_home;
        }
    }
    if let Some(environment) = config
        .as_object_mut()
        .and_then(|config| config.remove("env"))
    {
        let current = config["environments"]["java"]["env"].take();
        let mut combined = environment;
        if !current.is_null() {
            merge(&mut combined, current);
        }
        config["environments"]["java"]["env"] = combined;
    }
    let tasks_path = directory.join("tasks.json");
    let tasks = read(&tasks_path)?;
    if !tasks.is_null() && !tasks.is_array() {
        return Err(io::Error::other("tasks.json 必须是任务数组，旧配置未迁移"));
    }
    let mut remaining_tasks = None;
    if let Some(tasks) = tasks.as_array() {
        let mut remaining = Vec::new();
        let mut imported = Vec::new();
        for (index, task) in tasks.iter().enumerate() {
            let command = task["command"].as_str().unwrap_or_default();
            let executable = command.replace('\\', "/");
            let executable = executable
                .trim_matches('"')
                .rsplit('/')
                .next()
                .unwrap_or_default();
            let powershell = ["powershell.exe", "powershell", "pwsh.exe", "pwsh"]
                .iter()
                .any(|name| executable.eq_ignore_ascii_case(name));
            let expected = root
                .join(".zed/run.ps1")
                .to_string_lossy()
                .replace('\\', "/");
            let is_launcher = |value: &str| {
                let value = value.replace('\\', "/");
                let value = value.trim_matches(['"', '\'']);
                value == expected
                    || [
                        ".zed/run.ps1",
                        "$ZED_WORKTREE_ROOT/.zed/run.ps1",
                        "${ZED_WORKTREE_ROOT}/.zed/run.ps1",
                    ]
                    .contains(&value)
            };
            let owned = is_launcher(command)
                || (powershell
                    && task["args"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .any(is_launcher));
            if owned {
                imported.push(index);
            } else {
                remaining.push(task.clone());
            }
        }
        // Do not take ownership of unrelated, user-authored Zed tasks.
        if !imported.is_empty() {
            retired.push(tasks_path.clone());
            if !remaining.is_empty() {
                remaining_tasks = Some(remaining);
            }
            let script = directory.join("run.ps1");
            if script.is_file() {
                if script.is_symlink() {
                    return Err(io::Error::other("不能自动迁移符号链接形式的旧运行脚本"));
                }
                retired.push(script);
            }
        }
    }
    let state = directory.join(".run-state.json");
    if state.is_file() {
        read(&state)?;
        retired.push(state);
    }
    protect_migrated_settings(root)?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(io::Error::other)?
        .as_nanos();
    let backup = cache.join(format!("migration-{stamp}-{}", std::process::id()));
    fs::create_dir_all(&backup)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&backup, fs::Permissions::from_mode(0o700))?;
    }
    for source in &retired {
        let name = source
            .file_name()
            .ok_or_else(|| io::Error::other("运行配置文件名无效"))?;
        let bytes = fs::read(source)?;
        let copied = backup.join(name);
        fs::write(&copied, &bytes)?;
        if fs::read(&copied)? != bytes {
            return Err(io::Error::other("运行配置备份校验失败，未清理旧配置"));
        }
    }
    let temporary = directory.join(format!(".zed-runner-{stamp}.tmp"));
    let bytes = serde_json::to_vec_pretty(&config).map_err(io::Error::other)?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temporary)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    drop(file);
    // Publish without overwriting a configuration another process may have created.
    if let Err(error) = fs::hard_link(&temporary, &destination) {
        fs::remove_file(&temporary)?;
        if error.kind() == io::ErrorKind::AlreadyExists {
            return read(&destination);
        }
        return Err(error);
    }
    fs::remove_file(&temporary)?;
    for source in retired {
        if source == tasks_path {
            if let Some(tasks) = &remaining_tasks {
                fs::write(
                    &source,
                    serde_json::to_vec_pretty(tasks).map_err(io::Error::other)?,
                )?;
                continue;
            }
        }
        fs::remove_file(source)?;
    }
    Ok(config)
}

#[cfg(test)]
#[path = "tests/config_migration.rs"]
mod tests;
