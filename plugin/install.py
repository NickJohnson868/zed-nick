"""Build-machine installer for personal plugins; packaged users need no script runtime."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tarfile
import tempfile
import urllib.request

HERE = Path(__file__).resolve().parent
REPOSITORY = HERE.parent


def run(*arguments, env=None):
    subprocess.run([str(argument) for argument in arguments], cwd=REPOSITORY, env=env, check=True)


def extract(archive, destination):
    destination = destination.resolve()
    with tarfile.open(archive, "r:gz") as package:
        for member in package.getmembers():
            path = Path(member.name.replace("\\", "/"))
            if path.is_absolute() or ".." in path.parts or re.match(r"^[A-Za-z]:", member.name):
                raise ValueError("Archive contains an external path")
            (destination / path).resolve().relative_to(destination)
            if not member.isfile() and not member.isdir():
                raise ValueError("Archive links and special files are not supported")
        package.extractall(destination, filter="data")


def copy_runtime(source, destination):
    for parent in [destination, *destination.parents]:
        if parent.is_symlink():
            raise ValueError(f"Preserving development link: {parent}")
    for item in source.rglob("*"):
        if item.is_symlink():
            raise ValueError("Runtime packages must not contain links")
        relative = item.relative_to(source)
        target = destination / relative
        if target.is_symlink():
            raise ValueError(f"Preserving development link: {target}")
        if item.is_dir():
            target.mkdir(parents=True, exist_ok=True)
        else:
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(item, target)


def install_community(extensions, cache):
    cache.mkdir(parents=True, exist_ok=True)
    for extension in json.loads((HERE / "community.json").read_text())["extensions"]:
        identifier, version = extension["id"], extension["version"]
        if not re.fullmatch(r"[a-z0-9-]+", identifier) or not re.fullmatch(r"\d+\.\d+\.\d+", version):
            raise ValueError("Invalid extension identity")
        url = f"https://api.zed.dev/extensions/{identifier}/{version}/download"
        if extension["download"] != url:
            raise ValueError("Expected an official extension URL")
        archive = cache / f"{identifier}-{version}.tar.gz"
        if not archive.is_file():
            with urllib.request.urlopen(url, timeout=60) as response, archive.open("wb") as output:
                shutil.copyfileobj(response, output)
        if hashlib.sha256(archive.read_bytes()).hexdigest().lower() != extension["sha256"].lower():
            raise ValueError(f"Checksum mismatch: {identifier}")
        with tempfile.TemporaryDirectory(dir=cache) as stage:
            stage = Path(stage)
            extract(archive, stage)
            metadata = (stage / "extension.toml").read_text()
            for key, value in [("id", identifier), ("version", version)]:
                if not re.search(rf'^\s*{key}\s*=\s*"{re.escape(value)}"', metadata, re.M):
                    raise ValueError("Unexpected extension manifest")
            copy_runtime(stage, extensions / "installed" / identifier)


def java_tasks(destination, windows=None):
    tasks = json.loads((HERE / "zed-java-fork/languages/java/tasks.json").read_text(encoding="utf-8"))
    use_windows = os.name == "nt" if windows is None else windows
    if use_windows:
        common = (HERE / "zed-java-fork/scripts/java_task_windows.ps1").read_text(encoding="utf-8")
        for task, mode in zip(tasks, ("main", "method", "class", "all", "cache"), strict=True):
            task["command"] = common + "\ntry { Invoke-JavaTask '" + mode + "'; exit 0 } catch { [Console]::Error.WriteLine($_.Exception.Message); exit 1 }"
            task["shell"] = {"with_arguments": {"program": "powershell.exe", "args": ["-NoLogo", "-NoProfile", "-NonInteractive"]}}
            task["show_command"] = False
    directory = destination / "languages/java"
    directory.mkdir(parents=True, exist_ok=True)
    (directory / "tasks.json").write_text(json.dumps(tasks, ensure_ascii=False, indent=2), encoding="utf-8")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--extensions-directory", type=Path, required=True)
    parser.add_argument("--build-directory", type=Path, default=REPOSITORY / ".local-plugin-build")
    parser.add_argument("--extension-builder", type=Path)
    parser.add_argument("--skip-build", action="store_true")
    parser.add_argument("--skip-community", action="store_true")
    parser.add_argument("--community-only", action="store_true")
    options = parser.parse_args()
    extensions = options.extensions_directory.resolve()
    build = options.build_directory.resolve()
    build.mkdir(parents=True, exist_ok=True)
    if options.community_only:
        install_community(extensions, build / "community")
        return
    host = next(line.removeprefix("host: ") for line in subprocess.check_output(["rustc", "-vV"], text=True).splitlines() if line.startswith("host: "))
    suffix = ".exe" if "windows" in host else ""
    target = build / "target"
    environment = dict(os.environ, CARGO_TARGET_DIR=str(target))
    builder = options.extension_builder or target / host / "release" / ("zed-extension" + suffix)
    if not options.skip_build:
        run("rustup", "target", "add", "wasm32-wasip2")
        if not builder.is_file():
            run("cargo", "build", "--locked", "--release", "--target", host, "-p", "extension_cli", "--bin", "zed-extension", env=environment)
        run("cargo", "build", "--locked", "--release", "--target", host, "--manifest-path", HERE / "zed-java-fork/Cargo.toml", "-p", "java-lsp-proxy", "-p", "gradle-lsp-bridge", env=environment)
        run("cargo", "build", "--locked", "--release", "--target", host, "--manifest-path", HERE / "zed-mybatis-jump/Cargo.toml", "-p", "mybatis-jump-lsp", env=environment)
        run("cargo", "build", "--locked", "--release", "--target", "wasm32-wasip2", "--manifest-path", HERE / "zed-mybatis-jump/Cargo.toml", "-p", "zed-mybatis-jump", env=environment)
        run(builder, "--source-dir", HERE / "zed-java-fork", "--output-dir", build / "java", "--scratch-dir", build / "scratch", env=environment)
    binaries = [("java", "java-lsp-proxy"), ("java", "gradle-lsp-bridge"), ("mybatis-jump", "mybatis-jump-lsp")]
    archive = build / "java/archive.tar.gz"
    wasm = target / "wasm32-wasip2/release/zed_mybatis_jump.wasm"
    required = [archive, wasm, *(target / host / "release" / (name + suffix) for _, name in binaries)]
    if any(not path.is_file() for path in required):
        raise ValueError(f"Missing native plugin artifacts for {host}")
    with tempfile.TemporaryDirectory(dir=build) as temporary:
        stage = Path(temporary)
        (stage / "java").mkdir()
        extract(archive, stage / "java")
        java_tasks(stage / "java")
        runner = stage / "zed-runner"
        runner.mkdir()
        for name in ["extension.toml", "run-provider.ps1", "config-template.ps1", "run-environments.template.json", "scripts"]:
            source = HERE / "zed-runner" / name
            if source.is_dir():
                shutil.copytree(source, runner / name)
            else:
                shutil.copy2(source, runner / name)
        mybatis = stage / "mybatis-jump"
        (mybatis / "bin").mkdir(parents=True)
        shutil.copy2(HERE / "zed-mybatis-jump/extension.toml", mybatis / "extension.toml")
        shutil.copy2(wasm, mybatis / "extension.wasm")
        shutil.copy2(target / host / "release" / ("mybatis-jump-lsp" + suffix), mybatis / "bin" / ("mybatis-jump-lsp" + suffix))
        for identifier in ["java", "mybatis-jump", "zed-runner"]:
            copy_runtime(stage / identifier, extensions / "installed" / identifier)
        for identifier, name in binaries:
            destination = extensions / "work" / identifier / "bin"
            destination.mkdir(parents=True, exist_ok=True)
            with tempfile.TemporaryDirectory(dir=build) as temporary_binary:
                prepared = Path(temporary_binary) / (name + suffix)
                shutil.copy2(target / host / "release" / (name + suffix), prepared)
                if not suffix:
                    prepared.chmod(prepared.stat().st_mode | 0o111)
                copy_runtime(prepared.parent, destination)
    if not options.skip_community:
        install_community(extensions, build / "community")
    print(f"Installed native plugins for {host}; restart ZedNick manually to load them.")


if __name__ == "__main__":
    main()
