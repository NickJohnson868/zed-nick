# ZedNick plugins on Windows, macOS and Linux

Personal plugins: `zed-runner`, `zed-java-fork` (extension ID `java`),
and `zed-mybatis-jump`. IDEA Islands Dark 2026 is a built-in app theme, not a
plugin. Its generator and pinned sources live in `script/themes/idea-islands-2026`.

`community.json` lists the community HTML, SQL and XML extensions by repository,
version, official download URL and archive SHA-256. Their source is not vendored.
`install.py` installs personal and pinned community plugins without PowerShell;
`install.ps1` remains an optional Windows installer. Use `--skip-community`
only for isolated tests or when explicitly installing personal plugins alone.
Downloaded packages and compiler outputs stay in the ignored `.local-plugin-build`.
The Windows portable builder includes these plugin runtimes automatically; it
never copies a user's settings, accounts or language-server caches into the ZIP.
Debian amd64 packages built by `python3 script/build-deb.py --output ZedNick-amd64.deb`
also include all six plugins. The `zed-nick` launcher installs the bundled plugins
into the current user's extension directory on first launch and after package
upgrades. It respects `--user-data-dir` and preserves development links.
Building requires Python and the Linux build dependencies; installed users do not
need Python or PowerShell for plugin installation or the editor's native runner.
Community plugins added to `community.json` are included automatically in both
the Windows ZIP and the Debian package.

The native app discovers and launches Java, Python, Go and npm entries without
PowerShell on Windows, macOS and Linux. The Maven panel also calls Maven directly.
Java workflows reuse the configured JDK and Maven; Python workflows reuse their
configured Python or uv. The generated environment template discovers tools on
that machine. Missing or invalid runtime paths are detected again when scanning
a project, so a Windows-only configuration can fall back to Linux or macOS tools.
Java needs a JDK, Maven projects need Maven/MVND, and Python projects use their
configured interpreter or virtual environment. Existing scan whitelists,
Git ignores, project trust and path checks remain in effect.

The native compatibility checks use JDK 25 and Chinese project paths. Older Java
launchers can still impose their own restrictions on emoji paths or non-ASCII
application arguments on Windows code pages; the runner does not change the
machine's system encoding. Use a JDK that supports the required characters.

## Build and install on the target machine

Java gutter tasks are packaged for their target OS: Unix uses its built-in
`/bin/sh`, and Windows uses its preinstalled Windows PowerShell 5.1. They do not
require PowerShell 7 or Git Bash. These optional language tasks do not run an
automatic `clean`; the toolbar and Maven panel use the native app path above.

Install Rust, the native C/C++ toolchain, Git and tar. The Java extension builder
also needs the grammar build dependencies used by Zed's extension tooling.
The build-machine installer (Python 3.12+) builds Java grammar WASM files using Zed's extension builder,
not just its Rust extension. Java proxies and the MyBatis server are compiled
for the current Rust host architecture, including Apple Silicon or Intel Macs.

macOS:

```bash
python3 plugin/install.py --extensions-directory "$HOME/Library/Application Support/Zed/extensions"
```

Linux (adjust for a custom `XDG_DATA_HOME`):

```bash
python3 plugin/install.py --extensions-directory "${XDG_DATA_HOME:-$HOME/.local/share}/zed/extensions"
```

Windows portable installation:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File plugin/install.ps1 -ExtensionsDirectory 'C:\path\to\Zed-Nick\portable-data\extensions'
```

Use the actual profile's `extensions` directory when launching with a custom
`--user-data-dir`. App display names do not change Zed's existing profile path.
The installer does not restart ZedNick or change user settings. It preserves
existing development links and stops rather than overwriting those links.
Load the plugins on the next manual restart. A user-configured
`lsp.mybatis-jump.binary.path` still takes precedence; remove an old Windows-only
override on a Mac to use the bundled native server. XML language support must
also be installed through Zed's extension browser.

For an existing `zed-extension` builder, pass `--extension-builder /path/to/zed-extension`.
`--skip-build` reuses artifacts only from the specified build directory and checks
that all current-host outputs exist before installing. No profile or credentials
are included in a plugin build.

## Maven tool window

ZedNick includes a native Maven panel because the extension API cannot register
custom dock panels. Its `M` button appears on the **right** when local Maven
projects are present. The panel follows the module/Lifecycle/Plugins/Profiles/
Dependencies tree used by IDEA. Single-click selects a goal; double-click or
the toolbar Run button executes it in Zed's task terminal. Refresh reads POM
files without executing Maven. Project trust is required to execute a goal.

The runner resolves `java.build_tool` in `.zed/run-environments.json` first,
then the project's Maven wrapper, then the local Maven/MVND installation. Java
environment variables are read from the same configuration. Skip tests, offline
mode, reactor dependencies (`-pl` / `-am`) and explicit profiles are supported;
panel options are stored per workspace. POM changes refresh the tree.

The tree shows declared POM dependencies and common/declared plugin goals;
it is not an effective-POM importer. `dependency:tree` resolves the actual
dependency tree in the terminal. Unknown plugins expose `help:describe`.

```json
"maven_panel": {
  "button": true,
  "dock": "right",
  "default_width": 320,
  "starts_open": false
}
```

Rebuild the app and update `zed-runner` to **0.3.0** for the native runtime. Updating only the extension does not add the
native panel to older app binaries.

## Runtime checks

```sh
python plugin/zed-runner/tests/native_smoke.py
```

These launch isolated fixture programs, not business applications. The GitHub
workflow `Personal plugin platforms` runs the checks on Windows, macOS and Linux.
