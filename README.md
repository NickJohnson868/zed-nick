> [!IMPORTANT]
> Remove this line to confirm you've reviewed this PR before submitting.

# Zed

[![Zed](https://img.shields.io/endpoint?url=https://raw.githubusercontent.com/zed-industries/zed/main/assets/badge/v0.json)](https://zed.dev)
[![CI](https://github.com/zed-industries/zed/actions/workflows/run_tests.yml/badge.svg)](https://github.com/zed-industries/zed/actions/workflows/run_tests.yml)

Welcome to Zed, a high-performance, multiplayer code editor from the creators of [Atom](https://github.com/atom/atom) and [Tree-sitter](https://github.com/tree-sitter/tree-sitter).

---

### Installation

On macOS, Linux, and Windows you can [download Zed directly](https://zed.dev/download) or install Zed via your local package manager ([macOS](https://zed.dev/docs/installation#macos)/[Linux](https://zed.dev/docs/linux#installing-via-a-package-manager)/[Windows](https://zed.dev/docs/windows#package-managers)).

Other platforms are not yet available:

- Web ([tracking discussion](https://github.com/zed-industries/zed/discussions/26195))

### Personal Windows build

Double-click [`build-portable.bat`](./build-portable.bat) (requires PowerShell 7). It builds with 8 parallel jobs, retains Cargo's cache, builds GUI and CLI separately, and assembles `release/` in the repository root. The BAT entry point invokes `build-portable.ps1` and forwards optional arguments. Double-click `release/Zed.exe` directly. The adjacent `portable-data` directory holds the profile; an explicit `--user-data-dir` takes precedence. The first profile import requires all Zed windows to be closed, and subsequent updates preserve the imported profile. Output is gitignored.

The title bar contains an entry selector, Run/Rerun and Stop. Entry discovery runs in the background against non-ignored project files. Per-language runtimes and environment variables are configured in `.zed/run-environments.json`; the menu can create/open the configuration. Java entries use incremental Maven `compile` plus module classpaths and skip Maven when the input/output cache is unchanged. Existing `primary-run` tasks are retained only when no Java entry is discovered. Private project environment configuration is preserved. Java runtime changes are source-only until the next portable build. This is initial discovery for common entry forms, not a complete language semantic index.

Project panel setting `project_panel.collapse_children_on_collapse` defaults to `true`. Collapsing a parent resets descendant expansion state; set it to `false` to preserve the previous behavior. The setting also appears as **Collapse Children With Parent** in Settings.

See [Windows portable build instructions](./BUILD-PORTABLE.md) for usage, run environment examples and current validation status.

### Developing Zed

- [Building Zed for macOS](./docs/src/development/macos.md)
- [Building Zed for Linux](./docs/src/development/linux.md)
- [Building Zed for Windows](./docs/src/development/windows.md)

### Contributing

See [CONTRIBUTING.md](./CONTRIBUTING.md) for ways you can contribute to Zed.

Also... we're hiring! Check out our [jobs](https://zed.dev/jobs) page for open roles.

### Licensing

Zed source code is licensed primarily under GPL-3.0-or-later, with Apache-2.0 components where marked.

License information for third party dependencies must be correctly provided for CI to pass.

We use [`cargo-about`](https://github.com/EmbarkStudios/cargo-about) to automatically comply with open source licenses. If CI is failing, check the following:

- Is it showing a `no license specified` error for a crate you've created? If so, add `publish = false` under `[package]` in your crate's Cargo.toml.
- Is the error `failed to satisfy license requirements` for a dependency? If so, first determine what license the project has and whether this system is sufficient to comply with this license's requirements. If you're unsure, ask a lawyer. Once you've verified that this system is acceptable add the license's SPDX identifier to the `accepted` array in `script/licenses/zed-licenses.toml`.
- Is `cargo-about` unable to find the license for a dependency? If so, add a clarification field at the end of `script/licenses/zed-licenses.toml`, as specified in the [cargo-about book](https://embarkstudios.github.io/cargo-about/cli/generate/config.html#crate-configuration).

## Sponsorship

Zed is developed by **Zed Industries, Inc.**, a for-profit company.

If you’d like to financially support the project, you can do so via GitHub Sponsors.
Sponsorships go directly to Zed Industries and are used as general company revenue.
There are no perks or entitlements associated with sponsorship.
