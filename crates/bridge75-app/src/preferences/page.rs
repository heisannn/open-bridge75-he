use gpui_kit::component::notification::Notification;
use gpui_kit::component::radio::RadioGroup;
use gpui_kit::component::{ActiveTheme as _, WindowExt as _, v_flex};
use gpui_kit::{
    App, IntoElement, ParentElement as _, RenderOnce, SharedString, Styled as _, Window, div,
};
use rust_i18n::t;

use super::{Appearance, Language, Preferences};
use crate::page::{self, Field, PageHeader};

/// Repeated save failures share one notification.
struct SaveFailed;

/// The app's own settings. Its state is the [`Preferences`] global.
#[derive(IntoElement)]
pub struct PreferencesPage;

impl PreferencesPage {
    fn language_label(language: Language) -> SharedString {
        match language {
            Language::System => t!("preferences.language.system"),
            Language::English => t!("preferences.language.english"),
            Language::Japanese => t!("preferences.language.japanese"),
        }
        .into()
    }

    fn appearance_label(appearance: Appearance) -> SharedString {
        match appearance {
            Appearance::System => t!("preferences.appearance.system"),
            Appearance::Light => t!("preferences.appearance.light"),
            Appearance::Dark => t!("preferences.appearance.dark"),
        }
        .into()
    }

    fn change(window: &mut Window, cx: &mut App, edit: impl FnOnce(&mut Preferences)) {
        if let Err(e) = Preferences::update(cx, window, edit) {
            let detail = SharedString::from(e.to_string());
            window.push_notification(
                Notification::error(t!("preferences.save_failed.body"))
                    .id::<SaveFailed>()
                    .title(t!("preferences.save_failed"))
                    .content(move |_, _, cx| {
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(detail.clone())
                            .into_any_element()
                    }),
                cx,
            );
        }
    }
}

impl RenderOnce for PreferencesPage {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let preferences = Preferences::global(cx).clone();
        let language_ix = Language::ALL
            .iter()
            .position(|l| *l == preferences.language);
        let appearance_ix = Appearance::ALL
            .iter()
            .position(|a| *a == preferences.appearance);

        v_flex()
            .w_full()
            .p(page::INSET)
            .gap_6()
            .child(PageHeader::new(
                t!("preferences.title"),
                t!("preferences.description"),
            ))
            .child(Field::new(
                t!("preferences.language"),
                RadioGroup::vertical("language")
                    .children(Language::ALL.map(Self::language_label))
                    .selected_index(language_ix)
                    .on_change(|ix: &usize, window, cx| {
                        let language = Language::ALL[*ix];
                        Self::change(window, cx, |p| p.language = language);
                    }),
            ))
            .child(Field::new(
                t!("preferences.appearance"),
                RadioGroup::vertical("appearance")
                    .children(Appearance::ALL.map(Self::appearance_label))
                    .selected_index(appearance_ix)
                    .on_change(|ix: &usize, window, cx| {
                        let appearance = Appearance::ALL[*ix];
                        Self::change(window, cx, |p| p.appearance = appearance);
                    }),
            ))
    }
}
