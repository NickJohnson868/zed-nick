# IDEA Islands Dark 2026 for Zed

This theme ports the official IntelliJ IDEA 2026.2.3 Islands Dark interface palette and editor color scheme to Zed. Java keywords, types, methods, fields, strings, numbers, comments, and annotations use the values from `IslandSchemeDark.xml`.

Source release: [JetBrains IntelliJ IDEA `idea/2026.2.3`](https://github.com/JetBrains/intellij-community/releases/tag/idea%2F2026.2.3). The pinned upstream files and their license/notice are in `upstream/`; run `python generate.py` to regenerate the Zed theme. Zed's built-in One Dark theme provides the output structure. Zed cannot reproduce all IntelliJ UI shapes, but the mapped colors come from the official source.

The installable extension contains `extension.toml` and `themes/idea-islands-dark-2026.json`. Enable `IDEA Islands Dark 2026` in Zed's theme selector. For richer Java symbol coloring, set `languages.Java.semantic_tokens` to `combined` while keeping the Java extension and JDTLS enabled.
