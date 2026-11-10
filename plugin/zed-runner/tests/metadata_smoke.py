"""Check package/config wiring without compiling or launching any project."""
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

PLUGIN = Path(__file__).resolve().parents[1]
REPOSITORY = PLUGIN.parents[1]


class NativeRunnerMetadata(unittest.TestCase):
    def test_no_powershell_runtime_scripts(self):
        self.assertEqual(list((PLUGIN / "scripts").glob("*.ps1")), [])
        self.assertFalse((PLUGIN / "run-provider.ps1").exists())
        source = (REPOSITORY / "crates/title_bar/src/run_entries.rs").read_text(encoding="utf-8")
        self.assertNotIn("fn provider_shell", source)
        self.assertNotIn('"-ExecutionPolicy"', source)

    def test_java_tasks_use_native_helper_on_both_shell_families(self):
        spec = importlib.util.spec_from_file_location("runner_install", PLUGIN.parent / "install.py")
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        with tempfile.TemporaryDirectory(prefix="runner-metadata-") as temporary:
            for windows, shell in [(True, "cmd.exe"), (False, "/bin/sh")]:
                directory = Path(temporary) / str(windows)
                module.java_tasks(directory, windows=windows)
                tasks = json.loads((directory / "languages/java/tasks.json").read_text(encoding="utf-8"))
                self.assertEqual(len(tasks), 5)
                for task, mode in zip(tasks, ["main", "method", "class", "all", "cache"], strict=True):
                    self.assertEqual(task["shell"], {"program": shell})
                    self.assertIn("$ZED_CUSTOM_java_task_runner", task["command"])
                    self.assertTrue(task["command"].endswith(f"--java-task {mode}"))
                    self.assertNotIn("powershell", task["command"].lower())
                    self.assertNotIn(".ps1", task["command"].lower())

    def test_single_configuration_and_native_defaults(self):
        config = json.loads((PLUGIN / "zed-runner.example.json").read_text(encoding="utf-8"))
        self.assertIn("scan_directories", config)
        self.assertIn("ignore_directories", config)
        self.assertEqual(set(config["environments"]), {"java", "python", "go", "javascript", "rust", "csharp"})
        shared = (PLUGIN / "config.rs").read_text(encoding="utf-8")
        self.assertIn('CONFIG_PATH: &str = ".zed/zed-runner.json"', shared)
        for file in ["crates/title_bar/src/run_entries.rs", "crates/maven_panel/src/maven_panel.rs"]:
            source = (REPOSITORY / file).read_text(encoding="utf-8")
            self.assertIn("runner_config::load", source)
            for retired in ["run-environments.json", "run-config.json", "runner-ignore.json"]:
                self.assertNotIn(retired, source)


if __name__ == "__main__":
    unittest.main()
