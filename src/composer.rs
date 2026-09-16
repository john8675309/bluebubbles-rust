use eframe::egui::{self, text::CCursor, text::CCursorRange, TextBuffer};

/// Consume send keys before the editor sees them, without interfering with IME
/// candidate confirmation. Shift+Enter is handled by the multiline editor.
pub fn take_send_key(ui: &egui::Ui, id: egui::Id) -> bool {
    let ime_id = id.with("ime_active");
    if !ui.is_enabled() || !ui.memory(|memory| memory.has_focus(id)) {
        ui.ctx().data_mut(|data| data.insert_temp(ime_id, false));
        return false;
    }
    let mut active = ui
        .ctx()
        .data_mut(|data| data.get_temp::<bool>(ime_id).unwrap_or(false));
    let mut composing = active;
    ui.input(|input| {
        for event in &input.events {
            if let egui::Event::Ime(event) = event {
                composing = true;
                match event {
                    egui::ImeEvent::Enabled | egui::ImeEvent::Preedit(_) => active = true,
                    egui::ImeEvent::Disabled | egui::ImeEvent::Commit(_) => active = false,
                }
            }
        }
    });
    ui.ctx().data_mut(|data| data.insert_temp(ime_id, active));
    if composing {
        return false;
    }
    let mut send = false;
    ui.input_mut(|input| {
        input.events.retain(|event| {
            if let egui::Event::Key {
                key: egui::Key::Enter,
                pressed: true,
                modifiers,
                repeat,
                ..
            } = event
            {
                if !modifiers.shift && !modifiers.alt {
                    send |= !repeat;
                    return false;
                }
            }
            true
        })
    });
    send
}

pub fn insert_emoji(text: &mut String, state: &mut egui::text_edit::TextEditState, emoji: &str) {
    let length = text.chars().count();
    let range = state
        .cursor
        .char_range()
        .map(|range| {
            let [start, end] = range.sorted();
            start.index..end.index
        })
        .unwrap_or(length..length);
    let start = range.start.min(length);
    text.delete_char_range(start..range.end.min(length));
    let inserted = text.insert_text(emoji, start);
    state
        .cursor
        .set_char_range(Some(CCursorRange::one(CCursor::new(start + inserted))));
}

/// The bundled egui emoji fonts render these without system font dependencies.
pub fn emoji_picker(ui: &mut egui::Ui) -> Option<&'static str> {
    let mut selected = None;
    ui.menu_button("☺ Emoji", |ui| {
        ui.label("Add an emoji");
        egui::Grid::new("emoji_grid")
            .spacing([4.0, 4.0])
            .show(ui, |ui| {
                for (index, (emoji, name)) in EMOJIS.iter().enumerate() {
                    if ui
                        .add(
                            egui::Button::new(egui::RichText::new(*emoji).size(24.0))
                                .min_size(egui::vec2(38.0, 38.0)),
                        )
                        .on_hover_text(*name)
                        .clicked()
                    {
                        selected = Some(*emoji);
                        ui.close_menu();
                    }
                    if index % 8 == 7 {
                        ui.end_row();
                    }
                }
            });
    });
    selected
}

const EMOJIS: &[(&str, &str)] = &[
    ("😀", "Grinning"),
    ("😃", "Happy"),
    ("😄", "Smiling"),
    ("😁", "Beaming"),
    ("😂", "Laughing"),
    ("😊", "Blushing"),
    ("😉", "Winking"),
    ("😍", "Heart eyes"),
    ("😘", "Kiss"),
    ("😎", "Cool"),
    ("😇", "Angel"),
    ("😋", "Delicious"),
    ("😜", "Playful"),
    ("😐", "Neutral"),
    ("😕", "Confused"),
    ("😢", "Sad"),
    ("😭", "Crying"),
    ("😡", "Angry"),
    ("😱", "Shocked"),
    ("😴", "Sleeping"),
    ("👍", "Thumbs up"),
    ("👎", "Thumbs down"),
    ("👏", "Applause"),
    ("🙌", "Celebration"),
    ("👋", "Wave"),
    ("👌", "Okay"),
    ("🙏", "Thanks"),
    ("💪", "Strong"),
    ("❤", "Heart"),
    ("💙", "Blue heart"),
    ("💕", "Hearts"),
    ("💔", "Broken heart"),
    ("🎉", "Party"),
    ("🎂", "Birthday"),
    ("🎁", "Gift"),
    ("🔥", "Fire"),
    ("✨", "Sparkles"),
    ("⭐", "Star"),
    ("☀", "Sun"),
    ("🌈", "Rainbow"),
    ("☕", "Coffee"),
    ("🍕", "Pizza"),
    ("🍻", "Cheers"),
    ("🎵", "Music"),
    ("🐶", "Dog"),
    ("🐱", "Cat"),
    ("🌹", "Rose"),
    ("✅", "Check mark"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emoji_replaces_unicode_selection_and_tracks_character_cursor() {
        let mut text = "Hi café 🌹!".to_owned();
        let mut state = egui::text_edit::TextEditState::default();
        state
            .cursor
            .set_char_range(Some(CCursorRange::two(CCursor::new(3), CCursor::new(9))));
        insert_emoji(&mut text, &mut state, "👍🏽");
        assert_eq!(text, "Hi 👍🏽!");
        assert_eq!(state.cursor.char_range().unwrap().primary.index, 5);
        insert_emoji(&mut text, &mut state, "😀");
        assert_eq!(text, "Hi 👍🏽😀!");
    }

    #[test]
    fn picker_emojis_have_bundled_glyphs() {
        let ctx = egui::Context::default();
        let _ = ctx.run(Default::default(), |ctx| {
            ctx.fonts(|fonts| {
                for (emoji, name) in EMOJIS {
                    assert!(
                        fonts.has_glyphs(&egui::FontId::proportional(24.0), emoji),
                        "Missing {name}"
                    );
                }
            });
        });
    }

    #[test]
    fn enter_sends_shift_enter_edits_and_ime_confirmation_never_sends() {
        let ctx = egui::Context::default();
        let id = egui::Id::new("test_composer");
        let mut draft = "Hello".to_owned();
        let mut frame = |mut events: Vec<egui::Event>| {
            events.insert(
                0,
                egui::Event::Key {
                    key: egui::Key::Enter,
                    physical_key: None,
                    pressed: false,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                },
            );
            let mut send = false;
            let _ = ctx.run(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        send = take_send_key(ui, id);
                        let output = egui::TextEdit::multiline(&mut draft)
                            .id(id)
                            .return_key(egui::KeyboardShortcut::new(
                                egui::Modifiers::SHIFT,
                                egui::Key::Enter,
                            ))
                            .show(ui);
                        output.response.request_focus();
                    });
                },
            );
            (send, draft.clone())
        };
        let enter = |modifiers| egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        };
        frame(vec![]);
        let (send, text) = frame(vec![enter(egui::Modifiers::NONE)]);
        assert!(send);
        assert_eq!(text, "Hello");
        let (send, text) = frame(vec![enter(egui::Modifiers::SHIFT)]);
        assert!(!send);
        assert!(text.contains('\n'));
        assert!(frame(vec![enter(egui::Modifiers::CTRL)]).0);
        assert!(
            !frame(vec![
                egui::Event::Ime(egui::ImeEvent::Enabled),
                enter(egui::Modifiers::NONE)
            ])
            .0
        );
        assert!(!frame(vec![enter(egui::Modifiers::NONE)]).0);
        assert!(
            !frame(vec![
                egui::Event::Ime(egui::ImeEvent::Commit("漢".into())),
                enter(egui::Modifiers::NONE)
            ])
            .0
        );
        assert!(frame(vec![enter(egui::Modifiers::NONE)]).0);
    }
}
