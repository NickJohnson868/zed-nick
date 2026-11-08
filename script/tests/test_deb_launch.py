import os
from pathlib import Path
import subprocess
import tempfile
import unittest

LAUNCHER = Path(__file__).resolve().parents[1] / "zed-nick-launch"


@unittest.skipUnless(os.name == "posix", "Debian launcher uses POSIX utilities")
class DebianLauncherTest(unittest.TestCase):
    def test_install_upgrade_custom_profile_and_link_preservation(self):
        with tempfile.TemporaryDirectory(prefix="zed-nick-launch-") as temporary:
            root = Path(temporary)
            installation = root / "中文 install"
            bundle = installation / "extensions"
            for identifier in ["java", "mybatis-jump", "zed-runner", "html", "sql", "xml"]:
                path = bundle / "installed" / identifier
                path.mkdir(parents=True)
                (path / "extension.toml").write_text(identifier)
            (bundle / "work/java/bin").mkdir(parents=True)
            (bundle / "work/java/bin/java-lsp-proxy").write_text("native")
            (bundle / ".bundle-version").write_text("first")
            executable = installation / "libexec/zed-editor"
            executable.parent.mkdir()
            executable.write_text('#!/bin/sh\nprintf "%s\\n" "$@" > "$LAUNCH_ARGUMENTS"\n')
            executable.chmod(0o755)
            data_home = root / "data"
            arguments = root / "arguments"
            environment = dict(os.environ, ZED_NICK_INSTALL_ROOT=str(installation),
                               XDG_DATA_HOME=str(data_home), LAUNCH_ARGUMENTS=str(arguments))
            subprocess.run(["sh", LAUNCHER, "folder with spaces"], env=environment, check=True)
            extensions = data_home / "zed/extensions"
            self.assertEqual(len(list((extensions / "installed").iterdir())), 6)
            self.assertEqual(arguments.read_text().strip(), "folder with spaces")
            java = extensions / "installed/java/extension.toml"
            java.write_text("local")
            subprocess.run(["sh", LAUNCHER], env=environment, check=True)
            self.assertEqual(java.read_text(), "local")
            (bundle / ".bundle-version").write_text("second")
            subprocess.run(["sh", LAUNCHER], env=environment, check=True)
            self.assertEqual(java.read_text(), "java")
            custom = root / "custom profile"
            subprocess.run(["sh", LAUNCHER, "--user-data-dir", custom], env=environment, check=True)
            self.assertTrue((custom / "extensions/installed/java/extension.toml").is_file())
            linked = root / "linked profile"
            (linked / "extensions/installed").mkdir(parents=True)
            (linked / "extensions/installed/java").symlink_to(bundle / "installed/java", target_is_directory=True)
            result = subprocess.run(["sh", LAUNCHER, f"--user-data-dir={linked}"], env=environment,
                                    text=True, capture_output=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("Preserving linked plugin", result.stderr)
            self.assertTrue((linked / "extensions/installed/java").is_symlink())


if __name__ == "__main__":
    unittest.main()
