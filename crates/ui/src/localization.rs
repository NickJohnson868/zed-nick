use gpui::App;
use i18n::Language;
use settings::{RegisterSetting, Settings, SettingsContent, SettingsStore};

#[derive(Clone, RegisterSetting)]
struct InterfaceLanguage(bool);

impl Settings for InterfaceLanguage {
    fn from_settings(content: &SettingsContent) -> Self {
        Self(content.ui_language == Some(settings::UiLanguage::SimplifiedChinese))
    }
}

fn selected_language(cx: &App) -> Language {
    if std::env::var_os("ZED_NICK_I18N_PSEUDO").is_some_and(|value| value == "1") {
        Language::Pseudo
    } else if InterfaceLanguage::get_global(cx).0 {
        Language::SimplifiedChinese
    } else {
        Language::English
    }
}

pub fn init(cx: &mut App) {
    InterfaceLanguage::register(cx);
    i18n::set_language(selected_language(cx));
    cx.observe_global::<SettingsStore>(|cx| {
        let language = selected_language(cx);
        if i18n::language() != language {
            i18n::set_language(language);
            cx.refresh_windows();
        }
    })
    .detach();
}

/// Localize menu display names while leaving action identities and shortcuts intact.
pub fn translate_menus(menus: &mut [gpui::Menu]) {
    for menu in menus {
        menu.name = i18n::translate(&menu.name);
        for item in &mut menu.items {
            match item {
                gpui::MenuItem::Action { name, .. } => *name = i18n::translate(name),
                gpui::MenuItem::Submenu(submenu) => translate_menus(std::slice::from_mut(submenu)),
                _ => {}
            }
        }
    }
}
