"""Generate the Zed theme from the pinned IntelliJ IDEA 2026.2.3 resources."""

import copy
import json
from pathlib import Path
import xml.etree.ElementTree as ET


ROOT = Path(__file__).resolve().parent
REPO = ROOT.parents[1]
SOURCE = ROOT / "upstream"
OUTPUT = ROOT / "themes" / "idea-islands-dark-2026.json"


def main() -> None:
    jetbrains = json.loads((SOURCE / "ManyIslandsDark.theme.json").read_text(encoding="utf-8"))
    scheme = ET.parse(SOURCE / "IslandSchemeDark.xml").getroot()
    zed = json.loads((REPO / "assets/themes/one/one.json").read_text(encoding="utf-8"))
    theme = copy.deepcopy(next(item for item in zed["themes"] if item["name"] == "One Dark"))
    theme["name"] = "IDEA Islands Dark 2026"
    theme["appearance"] = "dark"
    style = theme["style"]
    palette = jetbrains["colors"]

    def color(key: str) -> str:
        seen = set()
        while not key.startswith("#"):
            if key in seen or key not in palette:
                raise ValueError(f"Unresolved JetBrains color: {key}")
            seen.add(key)
            key = palette[key]
        return key.lower()

    attributes = {item.attrib["name"]: item for item in scheme.find("attributes")}

    def attribute(name: str, part: str = "FOREGROUND") -> str:
        element = attributes[name]
        child = element.find(f"./value/option[@name='{part}']")
        if child is None or not child.get("value"):
            inherited = element.get("baseAttributes")
            if inherited:
                return attribute(inherited, part)
            raise ValueError(f"No {part} for {name}")
        return "#" + child.attrib["value"].lower()

    ui = {
        "background": "layer-0-bg",
        "surface.background": "layer-1-bg",
        "elevated_surface.background": "layer-2-bg",
        "border": "layer-0-border",
        "border.variant": "layer-1-border",
        "border.focused": "accent-brand-border",
        "border.selected": "tab-selected-border-active",
        "element.background": "layer-1-bg",
        "element.hover": "layer-2-bg",
        "element.active": "accent-brand-bg-secondary",
        "element.selected": "selection-bg-active",
        "ghost_element.hover": "layer-2-bg",
        "ghost_element.active": "accent-brand-bg-secondary",
        "ghost_element.selected": "selection-bg-active",
        "text": "text-default",
        "text.muted": "text-muted",
        "text.placeholder": "text-secondary",
        "text.disabled": "text-disabled",
        "text.accent": "text-link",
        "icon": "text-default",
        "icon.muted": "text-muted",
        "icon.disabled": "text-disabled",
        "icon.accent": "text-link",
        "status_bar.background": "layer-1-bg",
        "title_bar.background": "layer-1-bg",
        "tab_bar.background": "layer-0-bg",
        "tab.inactive_background": "layer-0-bg",
        "tab.active_background": "tab-selected-bg-inactive",
        "toolbar.background": "layer-1-bg",
        "panel.background": "layer-1-bg",
        "panel.focused_border": "accent-brand-border",
        "pane.focused_border": "accent-brand-border",
        "terminal.background": "layer-0-bg",
        "terminal.foreground": "editor-text",
        "created": "green-100",
        "deleted": "red-100",
        "error": "red-100",
        "info": "blue-100",
        "modified": "blue-100",
        "success": "green-100",
        "warning": "yellow-100",
    }
    for key, source in ui.items():
        style[key] = color(source)

    style["editor.background"] = attribute("TEXT", "BACKGROUND")
    style["editor.foreground"] = attribute("TEXT")
    style["editor.gutter.background"] = color("editor-bg")
    style["editor.subheader.background"] = color("layer-1-bg")
    style["editor.active_line.background"] = color("layer-0-bg-inline")
    style["editor.line_number"] = color("text-secondary")
    style["editor.active_line_number"] = color("text-default")
    style["editor.invisible"] = color("gray-50")
    style["editor.wrap_guide"] = color("gray-40")
    style["editor.active_wrap_guide"] = color("gray-60")
    style["search.match_background"] = color("search-match-bg")

    syntax = style["syntax"]
    syntax_sources = {
        "attribute": "DEFAULT_METADATA",
        "boolean": "DEFAULT_KEYWORD",
        "comment": "DEFAULT_LINE_COMMENT",
        "comment.doc": "DEFAULT_DOC_COMMENT",
        "constant": "DEFAULT_CONSTANT",
        "constructor": "DEFAULT_CLASS_REFERENCE",
        "enum": "DEFAULT_CLASS_REFERENCE",
        "function": "DEFAULT_INSTANCE_METHOD",
        "function.definition": "DEFAULT_FUNCTION_DECLARATION",
        "keyword": "DEFAULT_KEYWORD",
        "number": "DEFAULT_NUMBER",
        "operator": "DEFAULT_OPERATION_SIGN",
        "property": "DEFAULT_INSTANCE_FIELD",
        "punctuation.bracket": "DEFAULT_BRACKETS",
        "punctuation.delimiter": "DEFAULT_COMMA",
        "string": "DEFAULT_STRING",
        "string.escape": "DEFAULT_VALID_STRING_ESCAPE",
        "type": "DEFAULT_CLASS_REFERENCE",
        "variable": "DEFAULT_IDENTIFIER",
        "variable.parameter": "DEFAULT_IDENTIFIER",
    }
    for key, source in syntax_sources.items():
        syntax[key] = {"color": attribute(source)}

    result = {
        "$schema": "https://zed.dev/schema/themes/v0.2.0.json",
        "name": "IDEA Islands 2026",
        "author": "NickJohnson868; colors from JetBrains IntelliJ IDEA 2026.2.3",
        "themes": [theme],
    }
    OUTPUT.parent.mkdir(parents=True, exist_ok=True)
    OUTPUT.write_text(json.dumps(result, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
