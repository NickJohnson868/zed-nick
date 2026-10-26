# ZedNick plugins on Windows, macOS and Linux

Personal plugins: `zed-runner`, `zed-java-fork` (extension ID `java`),
`zed-mybatis-jump`, and `zed-idea-islands-2026`. The ignored `zed-java/` directory
is a local checkout, not the source shipped by this repository.

The runner uses **PowerShell 7 (`pwsh`) on every platform**. Install it on the
target machine. macOS GUI launches also search `/opt/homebrew/bin/pwsh` and
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
pwsh -NoProfile -File plugin/install.ps1 -ExtensionsDirectory 'C:\path\to\Zed-Nick\portable-data\extensions'
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

## Runtime checks

```powershell
./plugin/zed-runner/tests/task_command_smoke.ps1 -JavaHome $env:JAVA_HOME
./plugin/zed-runner/tests/java_module_smoke.ps1 -JavaHome $env:JAVA_HOME
./plugin/zed-runner/tests/scan_smoke.ps1
```

These launch isolated fixture programs, not business applications. The GitHub
workflow `Personal plugin platforms` runs the checks on Windows, macOS and Linux.
