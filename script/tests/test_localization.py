"""Check built-in interface translations against the current source text."""
import json
from pathlib import Path
import re
import unittest


ROOT = Path(__file__).resolve().parents[2]
LITERAL = r'"(?:[^"\\]|\\.)*"'


def decode_literal(literal):
    literal = re.sub(r"\\u\{([0-9a-fA-F]+)\}", lambda match: chr(int(match[1], 16)), literal)
    return json.loads(literal)


class LocalizationCoverage(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.entries = json.loads(
            (ROOT / "assets/locales/zh-CN/strings/zed-nick.json").read_text(encoding="utf-8"),
            object_pairs_hook=list,
        )
        cls.translations = {}
        for path in sorted((ROOT / "assets/locales/zh-CN/strings").glob("*.json")):
            cls.translations.update(json.loads(path.read_text(encoding="utf-8")))

    def test_unique_nonempty_translations(self):
        self.assertEqual(len(self.entries), len(dict(self.entries)), "Duplicate translation keys")
        self.assertTrue(self.translations)
        for key, translated in self.entries:
            self.assertTrue(translated.strip(), key)
            self.assertNotIn("\ufffd", translated, key)
            self.assertEqual(
                set(re.findall(r"\$\{[^}]+\}", key)),
                set(re.findall(r"\$\{[^}]+\}", translated)),
                key,
            )

    def test_main_settings_titles_descriptions_and_sections(self):
        source = (ROOT / "crates/settings_ui/src/page_data.rs").read_text(encoding="utf-8")
        texts = {
            decode_literal(value)
            for value in re.findall(
                rf"(?:title:\s*|description:\s*|SectionHeader\()({LITERAL})", source
            )
        }
        self.assertGreater(len(texts), 1000)
        missing = sorted(texts - self.translations.keys())
        self.assertEqual(missing, [], "Settings added without a Chinese translation")

    def test_static_application_menu_labels(self):
        source = (ROOT / "crates/zed/src/zed/app_menus.rs").read_text(encoding="utf-8")
        labels = {
            decode_literal(value)
            for value in re.findall(
                rf"(?:MenuItem::(?:action|os_submenu)\(\s*|name:\s*)({LITERAL})", source
            )
        }
        # Product and tool names are deliberately not translated.
        missing = sorted(labels - self.translations.keys() - {"Zed Nick", "Maven", "Zed Twitter"})
        self.assertEqual(missing, [])

    def test_git_action_menu_display_sites(self):
        source = (ROOT / "crates/git_ui/src/git_panel.rs").read_text(encoding="utf-8")
        for name in ("commit_options_menu", "render_action_toolbar"):
            start = source.index(f"    fn {name}(")
            end = source.index("\n    fn ", start + 8)
            section = source[start:end]
            # A dictionary entry alone is insufficient: each displayed menu label
            # must actually pass through the translator.
            calls = re.findall(
                rf"(?:ContextMenuEntry::new|\.header|\.submenu_with_icon|\.action|"
                rf"\.toggleable_entry)\(\s*(i18n::(?:t!|translate)\(\s*)?({LITERAL})",
                section,
            )
            self.assertTrue(calls, name)
            for translated, literal in calls:
                label = decode_literal(literal)
                self.assertTrue(translated, f"Untranslated menu label: {label}")
                self.assertTrue(label in self.translations, f"Missing translation: {label}")
        self.assertIn("ContextMenuEntry::new(i18n::translate(label))", section)


    def test_catalog_sections_have_no_duplicate_keys(self):
        for section in ("strings", "actions", "schema"):
            seen = set()
            for path in sorted((ROOT / "assets/locales/zh-CN" / section).glob("*.json")):
                entries = json.loads(path.read_text(encoding="utf-8"), object_pairs_hook=list)
                for key, _ in entries:
                    self.assertNotIn(key, seen, f"Duplicate {section} key in {path}")
                    seen.add(key)
            self.assertTrue(seen, section)

    def test_named_placeholders_are_preserved(self):
        for key, translated in self.translations.items():
            self.assertEqual(set(re.findall(r"\{([A-Za-z_][A-Za-z_0-9]*)\}", key)), set(re.findall(r"\{([A-Za-z_][A-Za-z_0-9]*)\}", translated)), key)

    def test_static_i18n_calls_have_translations(self):
        missing = []
        for path in (ROOT / "crates").glob("*/src/**/*.rs"):
            if "i18n/src" in path.as_posix():
                continue
            source = path.read_text(encoding="utf-8")
            for literal in re.findall(rf"i18n::(?:t!|translate)\(\s*({LITERAL})", source):
                key = decode_literal(literal)
                if key not in self.translations:
                    missing.append((str(path.relative_to(ROOT)), key))
        self.assertEqual(missing, [])


if __name__ == "__main__":
    unittest.main()
