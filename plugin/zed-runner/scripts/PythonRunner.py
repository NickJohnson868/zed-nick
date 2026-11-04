"""Prepare a requirements project using its configured Python, without a shell runtime."""
import hashlib
import os
from pathlib import Path
import subprocess
import sys
import venv


def main():
    project = Path(sys.argv[1]).resolve(strict=True)
    script = sys.argv[2]
    environment = project / ".venv"
    python = environment / ("Scripts/python.exe" if os.name == "nt" else "bin/python")
    requirements = project / "requirements.txt"
    manifests = [project / name for name in ("pyproject.toml", "requirements.txt", "uv.lock")]
    has_requirements = requirements.is_file() and any(line.strip() and not line.lstrip().startswith("#") for line in requirements.read_text().splitlines())
    uv = os.environ.get("ZED_PYTHON_UV")
    def fingerprint():
        digest = hashlib.sha256(sys.executable.encode())
        for path in manifests:
            if path.is_file():
                digest.update(path.name.encode())
                digest.update(path.read_bytes())
        return digest.hexdigest()
    cache = environment / ".zed-dependencies-stamp"
    stamp = fingerprint()
    ready = python.is_file() and cache.is_file() and cache.read_text() == stamp
    if not ready and manifests[0].is_file():
        if not uv:
            raise RuntimeError("pyproject.toml requires the configured uv executable")
        variables = dict(os.environ, UV_PROJECT_ENVIRONMENT=str(environment))
        subprocess.run([uv, "sync", "--project", str(project), "--no-default-groups", "--inexact", "--python", sys.executable], check=True, env=variables)
    if not python.is_file():
        if uv:
            subprocess.run([uv, "venv", "--python", sys.executable, str(environment)], check=True)
        else:
            venv.EnvBuilder(with_pip=has_requirements).create(environment)
    if not ready:
        if has_requirements:
            command = [uv, "pip", "install", "--python", str(python), "-r", str(requirements)] if uv else [str(python), "-m", "pip", "install", "-r", str(requirements)]
            subprocess.run(command, check=True, cwd=project)
        cache.write_text(fingerprint())
    if script != "--prepare":
        raise SystemExit(subprocess.call([str(python), script, *sys.argv[3:]]))


if __name__ == "__main__":
    main()
