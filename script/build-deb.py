"""Build a Debian amd64 package with all managed plugin runtimes."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import tempfile
import tomllib

ROOT = Path(__file__).resolve().parent.parent


def run(*arguments, **kwargs):
    return subprocess.run([str(value) for value in arguments], check=True, **kwargs)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--target-directory", type=Path, default=ROOT / ".local-linux-target")
    parser.add_argument("--plugin-build-directory", type=Path, default=ROOT / ".local-plugin-build")
    parser.add_argument("--jobs", type=int, choices=(1, 2), default=2)
    parser.add_argument("--skip-build", action="store_true")
    parser.add_argument("--skip-plugin-build", action="store_true")
    options = parser.parse_args()
    if platform.system() != "Linux" or platform.machine() not in ("x86_64", "amd64"):
        raise ValueError("Build this package on Debian amd64")
    os.chdir(ROOT)
    target = options.target_directory.resolve()
    environment = dict(os.environ, CARGO_TARGET_DIR=str(target), CARGO_BUILD_JOBS=str(options.jobs),
                       CARGO_PROFILE_RELEASE_FAST_DEBUG="0", ZED_RELEASE_CHANNEL="stable")
    if not options.skip_build:
        for package, binary in [("zed", "zed"), ("cli", "cli")]:
            run("cargo", "build", "--locked", "--profile", "release-fast", "-p", package,
                "--bin", binary, "--jobs", options.jobs, env=environment)
    package_version = tomllib.loads((ROOT / "crates/zed/Cargo.toml").read_text())["package"]["version"]
    commit = os.environ.get("ZED_COMMIT_SHA") or subprocess.check_output(
        ["git", "rev-parse", "HEAD"], text=True).strip()
    version = f"{package_version}+git.{commit[:10]}"
    cache = ROOT / ".local-deb-build"
    cache.mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="package-", dir=cache) as temporary:
        stage = Path(temporary)
        stage.chmod(0o755)
        application = stage / "usr/lib/zed-nick"
        (application / "libexec").mkdir(parents=True)
        for source, destination in [("zed", "zed-editor"), ("cli", "cli")]:
            binary = target / "release-fast" / source
            run("llvm-objcopy", "--strip-unneeded", binary, application / "libexec" / destination)
            (application / "libexec" / destination).chmod(0o755)
        install_arguments = ["python3", ROOT / "plugin/install.py", "--extensions-directory",
                             application / "extensions", "--build-directory", options.plugin_build_directory.resolve()]
        if options.skip_plugin_build:
            install_arguments.append("--skip-build")
        run(*install_arguments, env=environment)
        extensions = application / "extensions"
        expected = {"java", "mybatis-jump", "zed-runner"} | {
            item["id"] for item in json.loads((ROOT / "plugin/community.json").read_text())["extensions"]}
        actual = {path.name for path in (extensions / "installed").iterdir() if path.is_dir()}
        if actual != expected:
            raise ValueError(f"Missing packaged plugins: {expected - actual}")
        digest = hashlib.sha256()
        for path in sorted(extensions.rglob("*")):
            if path.is_file():
                digest.update(path.relative_to(extensions).as_posix().encode())
                digest.update(path.read_bytes())
        (extensions / ".bundle-version").write_text(digest.hexdigest() + "\n")
        launcher = stage / "usr/bin/zed-nick"
        launcher.parent.mkdir(parents=True)
        shutil.copy2(ROOT / "script/zed-nick-launch", launcher)
        launcher.chmod(0o755)
        desktop = stage / "usr/share/applications/zed-nick.desktop"
        desktop.parent.mkdir(parents=True)
        desktop.write_text("[Desktop Entry]\nType=Application\nName=Zed Nick\n"
                           "Exec=zed-nick %U\nIcon=zed-nick\nTerminal=false\n"
                           "Categories=Development;IDE;TextEditor;\n"
                           "MimeType=text/plain;inode/directory;x-scheme-handler/zed;\n")
        icon = stage / "usr/share/icons/hicolor/512x512/apps/zed-nick.png"
        icon.parent.mkdir(parents=True)
        shutil.copy2(ROOT / "crates/zed/resources/app-icon.png", icon)
        documentation = stage / "usr/share/doc/zed-nick"
        documentation.mkdir(parents=True)
        shutil.copy2(ROOT / "LICENSE-GPL", documentation / "copyright")
        control = (f"Package: zed-nick\nVersion: {version}\nArchitecture: amd64\n"
                   "Maintainer: Zed Nick maintainers\nSection: editors\nPriority: optional\n"
                   "Description: Zed Nick editor with bundled Java, MyBatis and project runners\n")
        (stage / "debian").mkdir()
        (stage / "debian/control").write_text("Source: zed-nick\n\n" + control)
        native_binaries = [application / "libexec/zed-editor", application / "libexec/cli",
                           *sorted((extensions / "work").glob("*/bin/*"))]
        dependencies = subprocess.check_output(
            ["dpkg-shlibdeps", "-O", "--ignore-missing-info", *["-e" + str(path) for path in native_binaries]],
            cwd=stage, text=True).strip().removeprefix("shlibs:Depends=")
        (stage / "DEBIAN").mkdir()
        (stage / "DEBIAN/control").write_text(
            control + "Depends: " + dependencies
            + ", util-linux, ca-certificates, git, libvulkan1, libegl1, libxkbcommon0, libxkbcommon-x11-0, libxcb1, libwayland-client0\n"
            + "Recommends: mesa-vulkan-drivers, xdg-desktop-portal\n")
        shutil.rmtree(stage / "debian")
        output = options.output.resolve()
        output.parent.mkdir(parents=True, exist_ok=True)
        run("dpkg-deb", "--build", "--root-owner-group", "-Zxz", "-z6", stage, output)
        print(f"Debian amd64 package: {output}")
        print(f"SHA256: {hashlib.sha256(output.read_bytes()).hexdigest()}")


if __name__ == "__main__":
    main()
