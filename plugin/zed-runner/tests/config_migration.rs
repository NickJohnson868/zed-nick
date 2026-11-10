use super::*;

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> io::Result<Self> {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(io::Error::other)?
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("zed-runner-config-{}-{stamp}", std::process::id()));
        fs::create_dir(&root)?;
        fs::create_dir(root.join(".zed"))?;
        Ok(Self(root))
    }

    fn put(&self, name: &str, value: Value) -> io::Result<()> {
        fs::write(
            self.0.join(".zed").join(name),
            serde_json::to_vec(&value).map_err(io::Error::other)?,
        )
    }

    fn load(&self) -> io::Result<Value> {
        load_with_cache(&self.0, &self.0.join("test-cache"))
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir_all(&self.0) {
            eprintln!("Could not clean isolated runner fixture: {error}");
        }
    }
}

#[test]
fn migrates_without_losing_overrides_or_user_tasks() -> io::Result<()> {
    let fixture = Fixture::new()?;
    fixture.put(
        "run-config.json",
        json!({"jdkPath":"/old-jdk", "env":{"PRIVATE":"example","OVERRIDE":"old"}}),
    )?;
    fixture.put("run-environments.json", json!({"environments":{"java":{"runtime":"/chosen-jdk","env":{"OVERRIDE":"chosen"}}},"entries":[]}))?;
    fixture.put(
        "runner-ignore.json",
        json!({"scan_directories":["app"],"ignore_directories":["tools"]}),
    )?;
    let user_task = json!({"label":"user-task","command":"echo","args":["run.ps1"]});
    fixture.put("tasks.json", json!([
        {"label":"old runner","command":"powershell.exe","args":["-File","$ZED_WORKTREE_ROOT/.zed/run.ps1"]},
        user_task.clone()
    ]))?;
    fixture.put(".run-state.json", json!({"processId":123,"startTicks":456}))?;
    fs::write(fixture.0.join(".zed/run.ps1"), "# old generated launcher")?;
    let config = fixture.load()?;
    assert_eq!(config["environments"]["java"]["runtime"], "/chosen-jdk");
    assert_eq!(config["environments"]["java"]["env"]["PRIVATE"], "example");
    assert_eq!(config["environments"]["java"]["env"]["OVERRIDE"], "chosen");
    assert_eq!(config["scan_directories"], json!(["app"]));
    assert_eq!(config["ignore_directories"], json!(["tools"]));
    assert_eq!(
        read(&fixture.0.join(".zed/tasks.json"))?,
        json!([user_task])
    );
    assert!(!fixture.0.join(".zed/run-config.json").exists());
    assert!(!fixture.0.join(".zed/run.ps1").exists());
    assert!(!fixture.0.join(".zed/.run-state.json").exists());
    let backup = fs::read_dir(fixture.0.join("test-cache"))?
        .next()
        .ok_or_else(|| io::Error::other("missing backup"))??
        .path();
    assert_eq!(read(&backup.join(".run-state.json"))?["processId"], 123);
    assert_eq!(
        read(&backup.join("tasks.json"))?.as_array().map(Vec::len),
        Some(2)
    );
    assert_eq!(fixture.load()?, config);
    Ok(())
}

#[test]
fn invalid_legacy_configuration_is_left_untouched() -> io::Result<()> {
    let fixture = Fixture::new()?;
    fixture.put("run-config.json", json!({"env":{"KEEP":"example"}}))?;
    fixture.put("run-environments.json", json!({"environments":"invalid"}))?;
    assert!(fixture.load().is_err());
    assert!(fixture.0.join(".zed/run-config.json").is_file());
    assert!(!fixture.0.join(CONFIG_PATH).exists());
    Ok(())
}

#[test]
fn explicit_configuration_is_never_overwritten() -> io::Result<()> {
    let fixture = Fixture::new()?;
    fixture.put(
        "zed-runner.json",
        json!({"entries":[],"scan_directories":["chosen"]}),
    )?;
    fixture.put("runner-ignore.json", json!({"scan_directories":["old"]}))?;
    assert_eq!(fixture.load()?["scan_directories"], json!(["chosen"]));
    assert!(fixture.0.join(".zed/runner-ignore.json").is_file());
    Ok(())
}

#[test]
fn absent_configuration_has_no_filesystem_side_effects() -> io::Result<()> {
    let fixture = Fixture::new()?;
    assert!(fixture.load()?.is_null());
    assert!(!fixture.0.join(CONFIG_PATH).exists());
    assert!(!fixture.0.join("test-cache").exists());
    Ok(())
}

#[cfg(unix)]
#[test]
fn merged_private_configuration_is_owner_only() -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let fixture = Fixture::new()?;
    fixture.put("run-config.json", json!({"env":{"PRIVATE":"example"}}))?;
    fixture.load()?;
    assert_eq!(
        fs::metadata(fixture.0.join(CONFIG_PATH))?
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    Ok(())
}
