use std::{
    fmt::Display,
    iter,
    sync::atomic::{AtomicU8, AtomicU64, Ordering},
};

pub use gpui_shared_string::SharedString;

/// Translates a string literal into the current UI language, falling back to the English text.
///
/// ```ignore
/// t!("Save");
/// t!("{count} files changed", count = changed.len());
/// ```
#[macro_export]
macro_rules! t {
    ($text:literal $(,)?) => {{
        static CALL_SITE: $crate::CallSite = $crate::CallSite::new();
        CALL_SITE.translate($text)
    }};
    ($text:literal, $($name:ident = $value:expr),+ $(,)?) => {{
        static CALL_SITE: $crate::CallSite = $crate::CallSite::new();
        $crate::format(
            CALL_SITE.translate($text),
            &[$((stringify!($name), &$value as &dyn ::std::fmt::Display)),+],
        )
    }};
}

/// Like [`t!`], with a short context for English text that needs different translations in
/// different places.
///
/// ```ignore
/// tc!("git", "Stage");
/// ```
#[macro_export]
macro_rules! tc {
    ($context:literal, $text:literal $(,)?) => {{
        static CALL_SITE: $crate::CallSite = $crate::CallSite::new();
        CALL_SITE.translate_in_context($context, $text)
    }};
    ($context:literal, $text:literal, $($name:ident = $value:expr),+ $(,)?) => {{
        static CALL_SITE: $crate::CallSite = $crate::CallSite::new();
        $crate::format(
            CALL_SITE.translate_in_context($context, $text),
            &[$((stringify!($name), &$value as &dyn ::std::fmt::Display)),+],
        )
    }};
}

/// Marks English text that is stored now and translated with [`translate`] where it's shown.
#[macro_export]
macro_rules! n {
    ($text:literal $(,)?) => {
        $text
    };
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Language {
    #[default]
    English = 0,
    SimplifiedChinese = 1,
    /// Wraps every translated string in brackets, to find text that isn't translated yet.
    Pseudo = 2,
}

impl Language {
    pub fn from_locale(locale: &str) -> Self {
        if locale.to_ascii_lowercase().starts_with("zh") {
            Self::SimplifiedChinese
        } else {
            Self::English
        }
    }

    pub fn system() -> Self {
        system_locale().map_or(Self::English, |locale| Self::from_locale(&locale))
    }

    pub fn code(self) -> &'static str {
        match self {
            Self::English => "en",
            Self::SimplifiedChinese => "zh-CN",
            Self::Pseudo => "pseudo",
        }
    }

    fn catalog(self) -> Option<&'static Catalog> {
        match self {
            Self::SimplifiedChinese => Some(&SIMPLIFIED_CHINESE),
            Self::English | Self::Pseudo => None,
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn system_locale() -> Option<String> {
    sys_locale::get_locale()
}

// Read the global preference so application-specific AppleLanguages cannot mask it.
#[cfg(target_os = "macos")]
fn system_locale() -> Option<String> {
    use core_foundation::{
        array::CFArray,
        base::{CFType, TCFType},
        string::CFString,
    };
    use core_foundation_sys::preferences::{
        CFPreferencesCopyValue, kCFPreferencesAnyApplication, kCFPreferencesAnyHost,
        kCFPreferencesCurrentUser,
    };

    let key = CFString::from_static_string("AppleLanguages");
    let global_languages = unsafe {
        let value = CFPreferencesCopyValue(
            key.as_concrete_TypeRef(),
            kCFPreferencesAnyApplication,
            kCFPreferencesCurrentUser,
            kCFPreferencesAnyHost,
        );
        (!value.is_null()).then(|| CFType::wrap_under_create_rule(value))
    };
    global_languages
        .and_then(|value| value.downcast_into::<CFArray>())
        .and_then(|languages| {
            let first = languages.get(0)?;
            unsafe { CFType::wrap_under_get_rule(*first) }.downcast_into::<CFString>()
        })
        .map(|language| language.to_string())
        .or_else(sys_locale::get_locale)
}

static LANGUAGE: AtomicU8 = AtomicU8::new(Language::English as u8);

pub fn language() -> Language {
    match LANGUAGE.load(Ordering::Relaxed) {
        1 => Language::SimplifiedChinese,
        2 => Language::Pseudo,
        _ => Language::English,
    }
}

pub fn set_language(language: Language) {
    LANGUAGE.store(language as u8, Ordering::Relaxed);
}

/// Returns the current language's text for `english`, or `None` when it should show in English.
pub fn lookup(english: &str) -> Option<SharedString> {
    let language = language();
    if language == Language::Pseudo {
        return Some(pseudo(english));
    }
    let catalog = language.catalog()?;
    let index = catalog.find_string(english)?;
    Some(SharedString::new_static(catalog.strings[index].1))
}

pub fn translate(english: &str) -> SharedString {
    lookup(english).unwrap_or_else(|| english.to_owned().into())
}

pub fn translate_in_context(context: &'static str, english: &'static str) -> SharedString {
    CallSite::new().translate_in_context(context, english)
}

/// Remembers one `t!` or `tc!` call's translation, so repeated renders skip the table search.
#[doc(hidden)]
pub struct CallSite(AtomicU64);

const NOT_TRANSLATED: u32 = u32::MAX;

impl CallSite {
    pub const fn new() -> Self {
        Self(AtomicU64::new(0))
    }

    pub fn translate(&self, english: &'static str) -> SharedString {
        self.resolve(english, |catalog| catalog.find_string(english))
    }

    pub fn translate_in_context(
        &self,
        context: &'static str,
        english: &'static str,
    ) -> SharedString {
        self.resolve(english, |catalog| {
            catalog
                .find_string_in_context(context, english)
                .or_else(|| catalog.find_string(english))
        })
    }

    fn resolve(
        &self,
        english: &'static str,
        find: impl FnOnce(&'static Catalog) -> Option<usize>,
    ) -> SharedString {
        let language = language();
        let Some(catalog) = language.catalog() else {
            return match language {
                Language::Pseudo => pseudo(english),
                Language::English | Language::SimplifiedChinese => {
                    SharedString::new_static(english)
                }
            };
        };
        let tag = language as u64 + 1;
        let cached = self.0.load(Ordering::Relaxed);
        let index = if cached >> 32 == tag {
            cached as u32
        } else {
            let index = find(catalog)
                .and_then(|index| u32::try_from(index).ok())
                .unwrap_or(NOT_TRANSLATED);
            self.0
                .store(tag << 32 | u64::from(index), Ordering::Relaxed);
            index
        };
        match catalog.strings.get(index as usize) {
            Some((_, translation)) => SharedString::new_static(translation),
            None => SharedString::new_static(english),
        }
    }
}

impl Default for CallSite {
    fn default() -> Self {
        Self::new()
    }
}

/// Fills `{name}` placeholders in `template`. Unknown placeholders are left as they are.
pub fn format(template: SharedString, arguments: &[(&str, &dyn Display)]) -> SharedString {
    let mut result = String::with_capacity(template.len());
    let mut rest = template.as_str();
    while let Some(start) = rest.find('{') {
        result.push_str(&rest[..start]);
        let after_brace = &rest[start + 1..];
        let argument = after_brace.find('}').and_then(|end| {
            let name = &after_brace[..end];
            arguments
                .iter()
                .find(|(argument_name, _)| *argument_name == name)
                .map(|(_, value)| (value, end))
        });
        match argument {
            Some((value, end)) => {
                result.push_str(&value.to_string());
                rest = &after_brace[end + 1..];
            }
            None => {
                result.push('{');
                rest = after_brace;
            }
        }
    }
    result.push_str(rest);
    result.into()
}

/// The translated name of a command, like "切换注释" for `editor::ToggleComments`.
pub fn action_name(action: &str) -> Option<SharedString> {
    let language = language();
    if language == Language::Pseudo {
        return Some(pseudo(action));
    }
    let &(_, name, _) = language.catalog()?.find_action(action)?;
    Some(SharedString::new_static(name))
}

/// The translated name of a command namespace, like "编辑器" for `editor`.
pub fn action_namespace(namespace: &str) -> Option<SharedString> {
    let language = language();
    if language == Language::Pseudo {
        return Some(pseudo(namespace));
    }
    let namespaces = language.catalog()?.action_namespaces;
    let index = namespaces
        .binary_search_by(|(key, _)| (*key).cmp(namespace))
        .ok()?;
    Some(SharedString::new_static(namespaces[index].1))
}

pub fn action_description(action: &str) -> Option<SharedString> {
    let language = language();
    if language == Language::Pseudo {
        return Some(pseudo(action));
    }
    let &(_, _, description) = language.catalog()?.find_action(action)?;
    description.map(SharedString::new_static)
}

/// The translated description of a setting or command for the JSON schema, used by the hover
/// help in settings.json and keymap.json.
pub fn schema_description(english: &str) -> Option<SharedString> {
    let language = language();
    if language == Language::Pseudo {
        return Some(pseudo(english));
    }
    let descriptions = language.catalog()?.schema_descriptions;
    let index = descriptions
        .binary_search_by(|(key, _)| (*key).cmp(english))
        .ok()?;
    Some(SharedString::new_static(descriptions[index].1))
}

/// Whether `text` is `english` or its translation in any language.
pub fn is_any_translation_of(text: &str, english: &str) -> bool {
    text == english
        || [&SIMPLIFIED_CHINESE].into_iter().any(|catalog| {
            catalog
                .find_string(english)
                .is_some_and(|index| catalog.strings[index].1 == text)
        })
}

/// Translates English text that was built from one of `templates`, keeping the values that filled
/// its `{name}` placeholders. Returns `None` when no template matches.
pub fn translate_filled(text: &str, templates: &[&'static str]) -> Option<SharedString> {
    templates.iter().find_map(|template| {
        let values = match_template(template, text)?;
        let arguments = values
            .iter()
            .map(|(name, value)| (*name, value as &dyn Display))
            .collect::<Vec<_>>();
        Some(format(translate(template), &arguments))
    })
}

fn match_template<'a>(template: &'a str, text: &'a str) -> Option<Vec<(&'a str, &'a str)>> {
    let mut values = Vec::new();
    let mut template_rest = template;
    let mut text_rest = text;
    loop {
        let Some(start) = template_rest.find('{') else {
            return (template_rest == text_rest).then_some(values);
        };
        let literal = &template_rest[..start];
        text_rest = text_rest.strip_prefix(literal)?;
        let end = template_rest[start..].find('}')? + start;
        let name = &template_rest[start + 1..end];
        template_rest = &template_rest[end + 1..];
        let next_literal_end = template_rest.find('{').unwrap_or(template_rest.len());
        let next_literal = &template_rest[..next_literal_end];
        let value_end = if next_literal.is_empty() {
            if template_rest.is_empty() {
                text_rest.len()
            } else {
                return None;
            }
        } else if next_literal_end == template_rest.len() {
            text_rest.strip_suffix(next_literal)?.len()
        } else {
            text_rest.find(next_literal)?
        };
        values.push((name, &text_rest[..value_end]));
        text_rest = &text_rest[value_end..];
    }
}

fn pseudo(english: &str) -> SharedString {
    format!("⟦{english}⟧").into()
}

/// Tables generated from `assets/locales` by build.rs, each sorted by key.
struct Catalog {
    strings: &'static [(&'static str, &'static str)],
    actions: &'static [(&'static str, &'static str, Option<&'static str>)],
    action_namespaces: &'static [(&'static str, &'static str)],
    schema_descriptions: &'static [(&'static str, &'static str)],
}

impl Catalog {
    fn find_string(&self, english: &str) -> Option<usize> {
        self.strings
            .binary_search_by(|(key, _)| (*key).cmp(english))
            .ok()
    }

    fn find_string_in_context(&self, context: &str, english: &str) -> Option<usize> {
        self.strings
            .binary_search_by(|(key, _)| {
                key.bytes().cmp(
                    context
                        .bytes()
                        .chain(iter::once(b'|'))
                        .chain(english.bytes()),
                )
            })
            .ok()
    }

    fn find_action(
        &self,
        action: &str,
    ) -> Option<&'static (&'static str, &'static str, Option<&'static str>)> {
        let index = self
            .actions
            .binary_search_by(|(key, _, _)| (*key).cmp(action))
            .ok()?;
        self.actions.get(index)
    }
}

include!(concat!(env!("OUT_DIR"), "/catalogs.rs"));

/// Search in both display languages without mutating global language state.
pub fn lookup_chinese(english: &str) -> Option<SharedString> {
    let index = SIMPLIFIED_CHINESE.find_string(english)?;
    Some(SharedString::new_static(
        SIMPLIFIED_CHINESE.strings.get(index)?.1,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_preserves_existing_chinese_and_context_fallback() {
        assert!(lookup_chinese("Save").is_some());
        assert!(
            SIMPLIFIED_CHINESE
                .find_string_in_context("runner", "Run Selected Entry")
                .is_some()
        );
        assert!(
            SIMPLIFIED_CHINESE
                .strings
                .windows(2)
                .all(|pair| pair[0].0 < pair[1].0)
        );
    }

    #[test]
    fn formatting_preserves_values_and_unknown_placeholders() {
        assert_eq!(
            format(
                "{count}: {name} {missing} {".into(),
                &[("count", &2), ("name", &"{count}中文")]
            )
            .as_str(),
            "2: {count}中文 {missing} {"
        );
    }

    #[test]
    fn english_and_pseudo_modes_do_not_need_a_catalog() -> Result<(), Box<dyn std::error::Error>> {
        let call = CallSite::new();
        set_language(Language::English);
        assert_eq!(call.translate("Save").as_str(), "Save");
        set_language(Language::SimplifiedChinese);
        assert_eq!(
            call.translate("Save"),
            lookup_chinese("Save").ok_or("fixture translation missing")?
        );
        assert!(action_name("editor::GoToDefinition").is_some());
        assert!(
            schema_description(
                "Whether files open with a single click or a double click.\n\nDefault: single_click"
            )
            .is_some()
        );
        assert_eq!(
            translate("This text intentionally has no translation").as_str(),
            "This text intentionally has no translation"
        );
        assert_eq!(
            translate_in_context("missing context", "Save"),
            lookup_chinese("Save").ok_or("fixture translation missing")?
        );
        set_language(Language::Pseudo);
        assert_eq!(call.translate("Save").as_str(), "⟦Save⟧");
        set_language(Language::English);
        Ok(())
    }
}

pub fn search_text(english: &str) -> String {
    match lookup_chinese(english) {
        Some(translated) => format!("{english} {translated}"),
        None => english.to_owned(),
    }
}
