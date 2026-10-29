# ZedNick plugins on Windows, macOS and Linux

Personal plugins: `zed-runner`, `zed-java-fork` (extension ID `java`),
and `zed-mybatis-jump`. IDEA Islands Dark 2026 is a built-in app theme, not a
plugin. Its generator and pinned sources live in `script/themes/idea-islands-2026`.

`community.json` lists the community HTML, SQL and XML extensions by repository,
version, official download URL and archive SHA-256. Their source is not vendored.
`install.ps1` installs both personal and community plugins. Use `-SkipCommunity`
only for isolated tests or when explicitly installing personal plugins alone.
Downloaded packages and compiler outputs stay in the ignored `.local-plugin-build`.
The Windows portable builder includes these plugin runtimes automatically; it
never copies a user's settings, accounts or language-server caches into the ZIP.

On Windows the runner automatically finds the system PowerShell (5.1 or 7);
PowerShell 7 is not required to run or install the bundled plugins. macOS and
Linux use `pwsh`. macOS GUI launches also search `/opt/homebrew/bin/pwsh` and
`/usr/local/bin/pwsh`. The generated environment template discovers tools on
that machine; do not copy Windows executable paths into a Mac configuration.
Java needs a JDK, Maven projects need Maven/MVND, and Python projects use their
configured interpreter or virtual environment. Existing scan whitelists,
Git ignores, project trust and path checks remain in effect.

The compatibility checks use JDK 21 and Chinese project paths. Older Java
launchers can still impose their own restrictions on emoji paths or non-ASCII
application arguments on Windows code pages; the runner does not change the
machine's system encoding. Use a JDK that supports the required characters.

## Build and install on the target machine

Install Rust, the native C/C++ toolchain, Git and tar. The Java extension builder
also needs the grammar build dependencies used by Zed's extension tooling.
`install.ps1` builds the Java grammar WASM files using Zed's extension builder,
not just its Rust extension. Java proxies and the MyBatis server are compiled
for the current Rust host architecture, including Apple Silicon or Intel Macs.

macOS:

```bash
pwsh -NoProfile -File plugin/install.ps1 -ExtensionsDirectory "$HOME/Library/Application Support/Zed/extensions"
```

Linux (adjust for a custom `XDG_DATA_HOME`):

```bash
pwsh -NoProfile -File plugin/install.ps1 -ExtensionsDirectory "${XDG_DATA_HOME:-$HOME/.local/share}/zed/extensions"
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

For an existing `zed-extension` builder, pass `-ExtensionBuilder /path/to/zed-extension`.
`-SkipBuild` reuses artifacts only from the specified build directory and checks
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

Rebuild the app for this native feature and update `zed-runner` to **0.2.1** or
later for `scripts/run_maven.ps1`. Updating only the extension does not add the
native panel to older app binaries.

## Runtime checks

```powershell
./plugin/zed-runner/tests/task_command_smoke.ps1 -JavaHome $env:JAVA_HOME
./plugin/zed-runner/tests/java_module_smoke.ps1 -JavaHome $env:JAVA_HOME
./plugin/zed-runner/tests/scan_smoke.ps1
```

These launch isolated fixture programs, not business applications. The GitHub
workflow `Personal plugin platforms` runs the checks on Windows, macOS and Linux.
