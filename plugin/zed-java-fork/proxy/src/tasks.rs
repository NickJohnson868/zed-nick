#[path = "../../../zed-runner/config.rs"]
mod runner_config;

use serde_json::Value;
use std::{
    collections::BTreeMap,
    env, fs, io,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

fn canonical(path: &Path) -> io::Result<PathBuf> {
    let resolved = path.canonicalize()?;
    #[cfg(windows)]
    {
        let text = resolved.to_string_lossy();
        if let Some(unc) = text.strip_prefix(r"\\?\UNC\") {
            return Ok(PathBuf::from(format!(r"\\{}", unc)));
        }
        if let Some(local) = text.strip_prefix(r"\\?\") {
            return Ok(PathBuf::from(local));
        }
    }
    Ok(resolved)
}

fn error(message: &str) -> io::Error {
    io::Error::other(message)
}

fn find_tool(names: &[&str]) -> Option<PathBuf> {
    let mut directories: Vec<_> = env::var_os("PATH")
        .map(|paths| env::split_paths(&paths).collect())
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

fn configured_tool(value: &Value, names: &[&str]) -> Option<PathBuf> {
    let path = value.as_str().map(PathBuf::from)?;
    if !path.is_absolute() {
        return None;
    }
    if path.is_file() {
        return Some(path);
    }
    names
        .iter()
        .map(|name| path.join("bin").join(name))
        .find(|path| path.is_file())
}

fn variables(config: &Value) -> io::Result<BTreeMap<String, String>> {
    let mut variables = BTreeMap::new();
    for values in [&config["env"], &config["environments"]["java"]["env"]] {
        if values.is_null() {
            continue;
        }
        let values = values
            .as_object()
            .ok_or_else(|| error("Java env must be an object"))?;
        for (key, value) in values {
            let value = match value {
                Value::String(value) => value.clone(),
                Value::Null => String::new(),
                Value::Number(_) | Value::Bool(_) => value.to_string(),
                _ => return Err(error("Java environment values must be scalar")),
            };
            variables.insert(key.clone(), value);
        }
    }
    Ok(variables)
}

fn execute(
    program: &Path,
    arguments: &[String],
    root: &Path,
    variables: &BTreeMap<String, String>,
) -> io::Result<()> {
    let mut command;
    if cfg!(windows)
        && program
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                extension.eq_ignore_ascii_case("cmd") || extension.eq_ignore_ascii_case("bat")
            })
    {
        let mut parts = vec![program.to_string_lossy().into_owned()];
        parts.extend(arguments.iter().cloned());
        if parts
            .iter()
            .any(|part| part.contains(['"', '%', '\r', '\n']))
        {
            return Err(error(
                "Command argument cannot be safely represented by CMD",
            ));
        }
        let line = parts
            .iter()
            .map(|part| format!("\"{part}\""))
            .collect::<Vec<_>>()
            .join(" ");
        command = Command::new(env::var_os("COMSPEC").unwrap_or_else(|| "cmd.exe".into()));
        command.args(["/d", "/v:off", "/s", "/c"]);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.raw_arg(format!("\"{line}\""));
        }
        #[cfg(not(windows))]
        command.arg(line);
    } else if !cfg!(windows)
        && program
            .file_name()
            .is_some_and(|name| name == "mvnw" || name == "gradlew")
    {
        command = Command::new("/bin/sh");
        command.arg(program).args(arguments);
    } else {
        command = Command::new(program);
        command.args(arguments);
    }
    let status = command
        .current_dir(root)
        .envs(variables)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()?;
    if !status.success() {
        return Err(error(&format!("Task failed with exit status {status}")));
    }
    Ok(())
}

fn nearest(root: &Path, file: &Path, markers: &[&str]) -> Option<PathBuf> {
    file.parent()?
        .ancestors()
        .take_while(|directory| directory.starts_with(root))
        .find(|directory| {
            markers
                .iter()
                .any(|marker| directory.join(marker).is_file())
        })
        .map(Path::to_path_buf)
}

fn class_name() -> io::Result<String> {
    let class =
        env::var("ZED_CUSTOM_java_class_name").map_err(|_| error("No Java class is selected"))?;
    let outer = env::var("ZED_CUSTOM_java_outer_class_name").unwrap_or_default();
    let package = env::var("ZED_CUSTOM_java_package_name").unwrap_or_default();
    let class = if outer.is_empty() {
        class
    } else {
        format!("{outer}${class}")
    };
    Ok(if package.is_empty() {
        class
    } else {
        format!("{package}.{class}")
    })
}

fn classpath_jar(
    module: &Path,
    java: &Path,
    test: bool,
    variables: &BTreeMap<String, String>,
) -> io::Result<PathBuf> {
    let mut paths = vec![module.join("target/classes")];
    if test {
        paths.insert(0, module.join("target/test-classes"));
    }
    let dependencies = fs::read_to_string(module.join("target/zed-java-classpath.txt"))?;
    paths.extend(env::split_paths(dependencies.trim()));
    let mut urls = Vec::new();
    for path in paths {
        if !path.exists() {
            continue;
        }
        let path = canonical(&path)?;
        let mut uri = proxy_common::path_to_file_uri(&path);
        if path.is_dir() && !uri.ends_with('/') {
            uri.push('/');
        }
        urls.push(uri);
    }
    let mut line = format!("Class-Path: {}", urls.join(" "));
    let mut manifest = "Manifest-Version: 1.0\r\n".to_owned();
    while line.len() > 70 {
        // File URIs are percent-encoded ASCII, so byte boundaries are safe here.
        manifest.push_str(&line[..70]);
        manifest.push_str("\r\n");
        line = format!(" {}", &line[70..]);
    }
    manifest.push_str(&line);
    manifest.push_str("\r\n\r\n");
    let manifest_path = module.join("target/zed-java-classpath.mf");
    let jar_path = module.join("target/zed-java-main-classpath.jar");
    fs::write(&manifest_path, manifest)?;
    let jar = java
        .parent()
        .ok_or_else(|| error("JDK path has no parent"))?
        .join(if cfg!(windows) { "jar.exe" } else { "jar" });
    execute(
        &jar,
        &[
            "--create".into(),
            "--file".into(),
            jar_path.to_string_lossy().into_owned(),
            "--manifest".into(),
            manifest_path.to_string_lossy().into_owned(),
        ],
        module,
        variables,
    )?;
    Ok(jar_path)
}

fn clear_cache() -> io::Result<()> {
    let executable = env::current_exe()?;
    let directory = executable
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| error("Java work directory is unavailable"))?
        .canonicalize()?;
    for item in fs::read_dir(&directory)? {
        let item = item?;
        if !item.file_name().to_string_lossy().starts_with("jdtls-") {
            continue;
        }
        let kind = item.file_type()?;
        if !kind.is_dir() || kind.is_symlink() {
            continue;
        }
        let path = item.path().canonicalize()?;
        if path.parent() != Some(directory.as_path()) {
            return Err(error("Refusing external JDTLS cache path"));
        }
        fs::remove_dir_all(path)?;
    }
    println!("Default JDTLS cache cleared. Restart the language server.");
    Ok(())
}

pub fn run(mode: &str) -> io::Result<()> {
    if mode == "cache" {
        return clear_cache();
    }
    if !["main", "method", "class", "all"].contains(&mode) {
        return Err(error("Unknown Java task mode"));
    }
    let root = env::var_os("ZED_WORKTREE_ROOT")
        .map(PathBuf::from)
        .unwrap_or(env::current_dir()?);
    let root = canonical(&root)?;
    let config = runner_config::load(&root)?;
    let settings = &config["environments"]["java"];
    let mut variables = variables(&config)?;
    let java_name = if cfg!(windows) { "java.exe" } else { "java" };
    let configured_java = settings["runtime"]
        .as_str()
        .map(PathBuf::from)
        .filter(|home| home.is_absolute())
        .map(|home| home.join("bin").join(java_name))
        .filter(|path| path.is_file());
    let java = configured_java
        .or_else(|| {
            env::var_os("JAVA_HOME")
                .map(|home| PathBuf::from(home).join("bin").join(java_name))
                .filter(|path| path.is_file())
        })
        .or_else(|| {
            if !cfg!(target_os = "macos") {
                return None;
            }
            let output = Command::new("/usr/libexec/java_home").output().ok()?;
            if !output.status.success() {
                return None;
            }
            let home = std::str::from_utf8(&output.stdout).ok()?.trim();
            let java = PathBuf::from(home).join("bin/java");
            java.is_file().then_some(java)
        })
        .or_else(|| find_tool(&[java_name]))
        .ok_or_else(|| {
            error("JDK not found; configure environments.java.runtime in .zed/zed-runner.json")
        })?;
    let java = if cfg!(windows) {
        java
    } else {
        canonical(&java)?
    };
    let java_home = java
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| error("JDK directory is unavailable"))?;
    variables.insert("JAVA_HOME".into(), java_home.to_string_lossy().into_owned());
    let configured_path_key = variables
        .keys()
        .find(|key| {
            if cfg!(windows) {
                key.eq_ignore_ascii_case("PATH")
            } else {
                key.as_str() == "PATH"
            }
        })
        .cloned();
    let path_key = configured_path_key.unwrap_or_else(|| {
        env::vars()
            .find(|(key, _)| key.eq_ignore_ascii_case("PATH"))
            .map(|(key, _)| key)
            .unwrap_or_else(|| "PATH".into())
    });
    let inherited_path = variables
        .get(&path_key)
        .map(std::ffi::OsString::from)
        .or_else(|| env::var_os(&path_key))
        .unwrap_or_default();
    let mut path = vec![java_home.join("bin")];
    path.extend(env::split_paths(&inherited_path));
    variables.insert(
        path_key,
        env::join_paths(path)
            .map_err(io::Error::other)?
            .to_string_lossy()
            .into_owned(),
    );
    if cfg!(windows) {
        // Match JavaRunner's shared-console UTF-8 fix; JVM encoding alone is insufficient.
        let status = Command::new(env::var_os("COMSPEC").unwrap_or_else(|| "cmd.exe".into()))
            .args(["/d", "/c", "chcp 65001 >nul"])
            .status()?;
        if !status.success() {
            eprintln!("No shared console was available to set UTF-8.");
        }
    }
    let file = env::var_os("ZED_FILE")
        .map(PathBuf::from)
        .or_else(|| {
            [
                "pom.xml",
                "build.gradle",
                "build.gradle.kts",
                "settings.gradle",
                "settings.gradle.kts",
            ]
            .iter()
            .map(|name| root.join(name))
            .find(|path| path.is_file())
        })
        .ok_or_else(|| error("No Java file or build project is selected"))?;
    let file = canonical(&file)?;
    if !file.starts_with(&root) {
        return Err(error("Selected Java file is outside the project"));
    }
    let class = if mode == "all" {
        String::new()
    } else {
        class_name()?
    };
    if let Some(module) = nearest(&root, &file, &["pom.xml"]) {
        let reactor = module
            .ancestors()
            .take_while(|directory| directory.starts_with(&root))
            .filter(|directory| directory.join("pom.xml").is_file())
            .last()
            .unwrap_or(&module);
        let names: &[&str] = if cfg!(windows) {
            &["mvnd.exe", "mvn.cmd", "mvn.exe"]
        } else {
            &["mvnd", "mvn"]
        };
        let wrapper = reactor.join(if cfg!(windows) { "mvnw.cmd" } else { "mvnw" });
        let maven = configured_tool(&settings["build_tool"], names).or_else(|| wrapper.is_file().then_some(wrapper)).or_else(|| find_tool(names)).ok_or_else(|| error("Maven not found; configure environments.java.build_tool or provide Maven Wrapper"))?;
        let mut arguments = vec![
            "-f".into(),
            reactor.join("pom.xml").to_string_lossy().into_owned(),
            "-B".into(),
        ];
        if reactor != module.as_path() {
            arguments.extend([
                "-pl".into(),
                module
                    .strip_prefix(reactor)
                    .map_err(io::Error::other)?
                    .to_string_lossy()
                    .replace('\\', "/"),
                "-am".into(),
            ]);
        }
        if mode != "main" {
            arguments.extend([
                "test".into(),
                "-DfailIfNoTests=false".into(),
                "-Dsurefire.failIfNoSpecifiedTests=false".into(),
            ]);
            if mode != "all" {
                let selector = if mode == "method" {
                    format!(
                        "{}#{}",
                        class,
                        env::var("ZED_CUSTOM_java_method_name")
                            .map_err(|_| error("No Java test method selected"))?
                    )
                } else {
                    class
                };
                arguments.push(format!("-Dtest={selector}"));
            }
            return execute(&maven, &arguments, reactor, &variables);
        }
        let test = file
            .components()
            .any(|component| component.as_os_str() == "test");
        arguments.extend([
            "-DskipTests".into(),
            if test { "test-compile" } else { "compile" }.into(),
            "dependency:build-classpath".into(),
            "-Dmdep.outputFile=target/zed-java-classpath.txt".into(),
            format!("-DincludeScope={}", if test { "test" } else { "runtime" }),
            "-Dmdep.regenerateFile=true".into(),
        ]);
        execute(&maven, &arguments, reactor, &variables)?;
        let jar = classpath_jar(&module, &java, test, &variables)?;
        return run_java(&java, &jar, &class, settings, &root, &variables);
    }
    if let Some(module) = nearest(
        &root,
        &file,
        &[
            "build.gradle",
            "build.gradle.kts",
            "settings.gradle",
            "settings.gradle.kts",
        ],
    ) {
        let wrapper = root.join(if cfg!(windows) {
            "gradlew.bat"
        } else {
            "gradlew"
        });
        let names: &[&str] = if cfg!(windows) {
            &["gradle.bat", "gradle.exe"]
        } else {
            &["gradle"]
        };
        let gradle = wrapper
            .is_file()
            .then_some(wrapper)
            .or_else(|| find_tool(names))
            .ok_or_else(|| error("Gradle not found"))?;
        let prefix = module
            .strip_prefix(&root)
            .map_err(io::Error::other)?
            .to_string_lossy()
            .replace(['/', '\\'], ":");
        let prefix = if prefix.is_empty() {
            String::new()
        } else {
            format!(":{prefix}")
        };
        let mut arguments = vec![format!(
            "{prefix}:{}",
            if mode == "main" { "run" } else { "test" }
        )];
        if mode == "main" {
            arguments.push(format!("-PmainClass={class}"));
        }
        if mode == "method" || mode == "class" {
            let selector = if mode == "method" {
                format!(
                    "{}.{}",
                    class,
                    env::var("ZED_CUSTOM_java_method_name")
                        .map_err(|_| error("No Java test method selected"))?
                )
            } else {
                class
            };
            arguments.extend(["--tests".into(), selector]);
        }
        return execute(&gradle, &arguments, &root, &variables);
    }
    if mode != "main" {
        return Err(error("Tests require a Maven or Gradle project"));
    }
    let classes = root.join("target/zed-native-classes");
    fs::create_dir_all(&classes)?;
    let javac = java
        .parent()
        .ok_or_else(|| error("JDK directory is unavailable"))?
        .join(if cfg!(windows) { "javac.exe" } else { "javac" });
    let mut source = file
        .parent()
        .ok_or_else(|| error("Source directory is unavailable"))?;
    let package = env::var("ZED_CUSTOM_java_package_name").unwrap_or_default();
    for _ in package.split('.').filter(|part| !part.is_empty()) {
        source = source
            .parent()
            .ok_or_else(|| error("Invalid Java package path"))?;
    }
    if !source.starts_with(&root) {
        return Err(error("Java source root is outside the project"));
    }
    execute(
        &javac,
        &[
            "-encoding".into(),
            "UTF-8".into(),
            "-d".into(),
            classes.to_string_lossy().into_owned(),
            "-sourcepath".into(),
            source.to_string_lossy().into_owned(),
            file.to_string_lossy().into_owned(),
        ],
        &root,
        &variables,
    )?;
    run_java(&java, &classes, &class, settings, &root, &variables)
}

fn run_java(
    java: &Path,
    classpath: &Path,
    class: &str,
    settings: &Value,
    root: &Path,
    variables: &BTreeMap<String, String>,
) -> io::Result<()> {
    let read_arguments = |name: &str| -> io::Result<Vec<String>> {
        if settings[name].is_null() {
            return Ok(Vec::new());
        }
        settings[name]
            .as_array()
            .ok_or_else(|| error("Java arguments must be arrays"))?
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| error("Java arguments must be strings"))
            })
            .collect()
    };
    let mut arguments = vec![
        "-Dfile.encoding=UTF-8".into(),
        "-Dstdout.encoding=UTF-8".into(),
        "-Dstderr.encoding=UTF-8".into(),
    ];
    arguments.extend(read_arguments("jvm_args")?);
    arguments.extend([
        "-cp".into(),
        classpath.to_string_lossy().into_owned(),
        class.into(),
    ]);
    arguments.extend(read_arguments("args")?);
    execute(java, &arguments, root, variables)
}
