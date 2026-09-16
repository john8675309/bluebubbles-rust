use crate::app::App;
use bluebubbles_linux::model::Message;
use chrono::{Local, TimeZone};
use eframe::egui::{self, Align, Color32, Layout, RichText, ScrollArea, TextEdit};

use crate::theme::{palette, panel, BLUE};
use crate::widgets;

pub fn show(app: &mut App, ctx: &egui::Context) {
    let colors = palette(app.dark);
    egui::TopBottomPanel::top("toolbar")
        .frame(panel(colors.sidebar, 12))
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                widgets::mark(ui, 30.0);
                ui.label(RichText::new("BlueBubbles").size(19.0).strong());
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui
                        .button(if app.dark { "Light mode" } else { "Dark mode" })
                        .clicked()
                    {
                        app.dark = !app.dark;
                        crate::theme::apply(ctx, app.dark);
                    }
                    if app.connected {
                        if ui
                            .button("Disconnect")
                            .on_hover_text("Clears this session, including unsent drafts")
                            .clicked()
                        {
                            app.disconnect();
                        }
                        if ui
                            .add_enabled(!app.refreshing, egui::Button::new("Refresh"))
                            .clicked()
                        {
                            app.refresh(ctx);
                        }
                    }
                    if ui.button("Firebase").clicked() {
                        app.firebase_open = !app.firebase_open;
                        if app.firebase_open {
                            if let Some(config) = app.firebase_configs.get(&app.firebase_key()) {
                                let (preferences, at_login) =
                                    bluebubbles_linux::push::preferences(config);
                                app.push_previews = preferences.previews;
                                app.push_at_login = at_login;
                            }
                        }
                    }
                });
            });
        });
    egui::TopBottomPanel::bottom("status")
        .frame(panel(colors.sidebar, 8))
        .show(ctx, |ui| {
            ui.spacing_mut().interact_size.y = 14.0;
            ui.horizontal(|ui| {
                if app.busy {
                    ui.spinner();
                }
                ui.label(RichText::new(&app.status).size(11.0).color(colors.muted));
                if app.connected {
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(7.0, 7.0), egui::Sense::hover());
                    ui.painter().circle_filled(
                        rect.center(),
                        3.0,
                        if app.socket_online {
                            Color32::from_rgb(67, 185, 142)
                        } else {
                            Color32::from_rgb(210, 161, 75)
                        },
                    );
                    ui.label(
                        RichText::new(if app.socket_online {
                            "Live updates"
                        } else {
                            "Connecting live updates"
                        })
                        .size(11.0)
                        .color(colors.muted),
                    );
                }
                if let Some(time) = app.last_sync {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(
                            RichText::new(format!("Synced {}s ago", time.elapsed().as_secs()))
                                .size(11.0)
                                .color(colors.muted),
                        );
                    });
                }
            });
        });
    if let Some(error) = app.error.clone() {
        egui::TopBottomPanel::top("error")
            .frame(panel(colors.surface, 12))
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.colored_label(ui.visuals().error_fg_color, error);
                    if ui.small_button("Dismiss").clicked() {
                        app.error = None;
                    }
                });
            });
    }
    if app.firebase_open {
        firebase_panel(app, ctx);
    }
    if !app.connected {
        connection(app, ctx);
        return;
    }
    sidebar(app, ctx);
    conversation(app, ctx);
    if app.new_chat {
        new_chat(app, ctx);
    }
}

fn firebase_panel(app: &mut App, ctx: &egui::Context) {
    let mut open = app.firebase_open;
    egui::Window::new("Firebase")
        .open(&mut open)
        .default_width(490.0)
        .vscroll(true)
        .show(ctx, |ui| {
            ui.heading("Server URL recovery");
            ui.label("Find your Mac's current address when its tunnel URL changes.");
            ui.checkbox(
                &mut app.firebase_auto,
                "Recover the server URL after connection failures",
            );
            if let Some(config) = app.firebase_configs.get(&app.firebase_key()) {
                ui.label(format!("Project: {}", config.project_id));
                ui.label(if config.database_url.is_some() {
                    "Database: Realtime Database"
                } else {
                    "Database: Firestore"
                });
            }
            ui.label(&app.firebase_status);
            ui.add_enabled_ui(!app.firebase_busy, |ui| {
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .add_enabled(app.connected, egui::Button::new("Load from server"))
                        .clicked()
                    {
                        app.load_firebase(ctx);
                    }
                    if ui.button("Import google-services.json").clicked() {
                        app.import_firebase(ctx);
                    }
                    if ui
                        .add_enabled(
                            app.firebase_configs.contains_key(&app.firebase_key()) && !app.busy,
                            egui::Button::new("Recover URL now"),
                        )
                        .clicked()
                    {
                        app.resolve_firebase(ctx);
                    }
                });
            });
            ui.hyperlink_to("Firebase Console", "https://console.firebase.google.com/");
            ui.separator();
            ui.heading("Background notifications");
            ui.label("Receive Firebase messages even when this window is closed.");
            ui.checkbox(&mut app.push_previews, "Show message text in notifications");
            ui.checkbox(
                &mut app.push_at_login,
                "Start the notification receiver at login",
            );
            let config = app.firebase_configs.get(&app.firebase_key()).cloned();
            if let Some(config) = &config {
                ui.label(bluebubbles_linux::push::status(config));
            }
            ui.add_enabled_ui(!app.firebase_busy, |ui| {
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            app.connected && config.is_some(),
                            egui::Button::new("Enable / update notifications"),
                        )
                        .clicked()
                    {
                        app.enable_push(ctx);
                    }
                    if ui
                        .add_enabled(
                            config
                                .as_ref()
                                .is_some_and(bluebubbles_linux::push::enabled),
                            egui::Button::new("Disable notifications"),
                        )
                        .clicked()
                    {
                        app.disable_push(ctx);
                    }
                });
                ui.horizontal(|ui| {
                    if ui.button("Test desktop notification").clicked() {
                        app.test_notification(ctx);
                    }
                    if ui
                        .add_enabled(config.is_some(), egui::Button::new("Reset registration"))
                        .clicked()
                    {
                        app.reset_push(ctx);
                    }
                });
            });
        });
    app.firebase_open = open;
}

fn connection(app: &mut App, ctx: &egui::Context) {
    let colors = palette(app.dark);
    egui::CentralPanel::default()
        .frame(panel(colors.background, 20))
        .show(ctx, |ui| {
            let top_space = ((ui.available_height() - 540.0) / 2.0).max(0.0);
            ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.add_space(top_space);
                    ui.vertical_centered(|ui| {
                        widgets::mark(ui, 54.0);
                        ui.add_space(10.0);
                        ui.heading(
                            RichText::new("Your messages, on Linux.")
                                .size(30.0)
                                .strong(),
                        );
                        ui.label(
                            RichText::new("Stay close to your conversations, wherever you work.")
                                .color(colors.muted),
                        );
                        ui.add_space(22.0);
                        egui::Frame::new()
                            .fill(colors.surface)
                            .stroke(egui::Stroke::new(1.0_f32, colors.border))
                            .corner_radius(18)
                            .inner_margin(24)
                            .show(ui, |ui| {
                                ui.set_width(400.0);
                                ui.with_layout(Layout::top_down(Align::LEFT), |ui| {
                                    ui.label(
                                        RichText::new("Connect to your Mac").size(20.0).strong(),
                                    );
                                    ui.add_space(6.0);
                                    ui.add_enabled_ui(!app.busy, |ui| {
                                        ui.label(
                                            RichText::new("Server URL")
                                                .size(13.0)
                                                .color(colors.muted),
                                        );
                                        ui.add(
                                            TextEdit::singleline(&mut app.server)
                                                .hint_text("https://your-server.example.com")
                                                .margin(egui::vec2(12.0, 10.0))
                                                .desired_width(f32::INFINITY)
                                                .background_color(colors.background),
                                        );
                                        ui.add_space(2.0);
                                        ui.label(
                                            RichText::new("Server password")
                                                .size(13.0)
                                                .color(colors.muted),
                                        );
                                        let password = ui.add(
                                            TextEdit::singleline(&mut app.password)
                                                .password(true)
                                                .hint_text("Your BlueBubbles password")
                                                .margin(egui::vec2(12.0, 10.0))
                                                .desired_width(f32::INFINITY)
                                                .background_color(colors.background),
                                        );
                                        ui.add_space(8.0);
                                        let enter = password.lost_focus()
                                            && ui.input(|i| i.key_pressed(egui::Key::Enter));
                                        if ui
                                            .add_sized(
                                                [ui.available_width(), 44.0],
                                                egui::Button::new(
                                                    RichText::new("Connect").color(Color32::WHITE),
                                                )
                                                .fill(BLUE),
                                            )
                                            .clicked()
                                            || enter
                                        {
                                            app.connect(ctx);
                                        }
                                    });
                                    ui.add_space(10.0);
                                    ui.checkbox(
                                        &mut app.persist_history,
                                        RichText::new("Save history and drafts on this computer")
                                            .size(13.0),
                                    );
                                    ui.label(
                                RichText::new(
                                    "Your password stays in memory. Local history is optional.",
                                )
                                .size(11.0)
                                .color(colors.muted),
                            );
                                });
                            });
                        ui.add_space(18.0);
                        ui.hyperlink_to(
                            "Need help setting up your server?",
                            "https://bluebubbles.app/install/",
                        );
                    });
                });
        });
}

fn sidebar(app: &mut App, ctx: &egui::Context) {
    let colors = palette(app.dark);
    egui::SidePanel::left("chats")
        .resizable(true)
        .default_width(332.0)
        .width_range(270.0..=450.0)
        .frame(panel(colors.sidebar, 14))
        .show(ctx, |ui| {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.heading(RichText::new("Messages").size(26.0).strong());
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui
                        .add_enabled(
                            !app.busy,
                            egui::Button::new(RichText::new("+ New").color(BLUE))
                                .fill(colors.selection),
                        )
                        .clicked()
                    {
                        app.new_chat = true;
                    }
                });
            });
            ui.add_space(8.0);
            ui.add(
                TextEdit::singleline(&mut app.search)
                    .hint_text("Search conversations")
                    .margin(egui::vec2(12.0, 10.0))
                    .desired_width(f32::INFINITY),
            );
            ui.add_space(12.0);
            ui.label(
                RichText::new(format!("CONVERSATIONS  ·  {}", app.chats.len()))
                    .size(10.0)
                    .color(colors.muted),
            );
            ui.add_space(2.0);
            let query = app.search.to_lowercase();
            let mut select = None;
            ScrollArea::vertical().id_salt("chat_list").show(ui, |ui| {
                if app.chats.is_empty() {
                    ui.label(
                        RichText::new("Your conversations will appear here.").color(colors.muted),
                    );
                }
                for chat in &app.chats {
                    let title = app.chat_title(chat);
                    if !query.is_empty()
                        && !title.to_lowercase().contains(&query)
                        && !chat
                            .participants
                            .iter()
                            .any(|h| h.address.to_lowercase().contains(&query))
                    {
                        continue;
                    }
                    let selected = app.selected.as_deref() == Some(&chat.guid);
                    let preview = chat
                        .last_message
                        .as_ref()
                        .map(|message| {
                            format!(
                                "{}{}",
                                if message.is_from_me { "You: " } else { "" },
                                message.preview().replace('\n', " ")
                            )
                        })
                        .unwrap_or_else(|| "No messages yet".into());
                    let time = chat
                        .last_message
                        .as_ref()
                        .and_then(Message::activity_timestamp)
                        .and_then(|ms| Local.timestamp_millis_opt(ms).single())
                        .map(|d| {
                            if d.date_naive() == Local::now().date_naive() {
                                d.format("%-I:%M %p").to_string()
                            } else {
                                d.format("%b %-d").to_string()
                            }
                        })
                        .unwrap_or_default();
                    if ui
                        .push_id(&chat.guid, |ui| {
                            widgets::chat_row(ui, &title, &preview, &time, &chat.guid, selected)
                        })
                        .inner
                        .clicked()
                    {
                        select = Some(chat.guid.clone());
                    }
                }
            });
            if let Some(guid) = select {
                app.select(ctx, guid);
            }
        });
}

fn conversation(app: &mut App, ctx: &egui::Context) {
    let Some(guid) = app.selected.clone() else {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.centered_and_justified(|ui| {
                ui.heading("Choose a conversation or start a new message");
            });
        });
        return;
    };
    let colors = palette(app.dark);
    egui::TopBottomPanel::top("conversation_header")
        .frame(panel(colors.background, 20))
        .show(ctx, |ui| {
            if let Some(chat) = app.chats.iter().find(|c| c.guid == guid) {
                let title = app.chat_title(chat);
                ui.horizontal(|ui| {
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(46.0, 46.0), egui::Sense::hover());
                    widgets::avatar(ui, rect.center(), &title, &guid, 46.0);
                    ui.vertical(|ui| {
                        ui.add(
                            egui::Label::new(RichText::new(&title).size(23.0).strong()).truncate(),
                        );
                        let detail = if app.typing.contains_key(&guid) {
                            "Typing…".into()
                        } else {
                            chat.participants
                                .iter()
                                .map(|h| h.address.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        };
                        ui.add(
                            egui::Label::new(RichText::new(detail).size(12.0).color(colors.muted))
                                .truncate(),
                        );
                    });
                });
            }
            ui.add_space(4.0);
            ui.add(
                TextEdit::singleline(&mut app.message_search)
                    .hint_text("Find in this conversation")
                    .font(egui::FontId::proportional(13.0))
                    .margin(egui::vec2(12.0, 8.0))
                    .desired_width(ui.available_width().min(360.0)),
            );
        });
    composer(app, ctx, &guid);
    let mut download = None;
    egui::CentralPanel::default()
        .frame(panel(colors.background, 22))
        .show(ctx, |ui| {
            let scroll_bottom = std::mem::take(&mut app.scroll_to_bottom);
            ScrollArea::vertical()
                .id_salt(("messages", &guid))
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    if app.more_messages.get(&guid).copied().unwrap_or(false)
                        && ui
                            .add_enabled(
                                !app.messages_loading(&guid),
                                egui::Button::new("Load older messages"),
                            )
                            .clicked()
                    {
                        app.load_older(ctx);
                    }
                    if let Some(messages) = app.messages.get(&guid) {
                        if messages.is_empty() {
                            ui.label("No messages in this conversation yet.");
                        }
                        let query = app.message_search.to_lowercase();
                        let mut previous_day = String::new();
                        for message in messages {
                            if !query.is_empty()
                                && !message.preview().to_lowercase().contains(&query)
                            {
                                continue;
                            }
                            let day = message
                                .date_created
                                .and_then(|ms| Local.timestamp_millis_opt(ms).single())
                                .map(|d| {
                                    if d.date_naive() == Local::now().date_naive() {
                                        "Today".into()
                                    } else {
                                        d.format("%A, %B %-d, %Y").to_string()
                                    }
                                })
                                .unwrap_or_default();
                            if day != previous_day {
                                widgets::day_separator(ui, &day);
                                previous_day = day;
                            }
                            let sender = if app
                                .chats
                                .iter()
                                .any(|chat| chat.guid == guid && chat.participants.len() > 1)
                                && !message.is_from_me
                            {
                                message.handle.as_ref().map(|handle| {
                                    app.contacts
                                        .get(&bluebubbles_linux::api_actions::normalize_address(
                                            &handle.address,
                                        ))
                                        .cloned()
                                        .unwrap_or_else(|| handle.address.clone())
                                })
                            } else {
                                None
                            };
                            message_bubble(ui, message, sender.as_deref(), app.busy, &mut download);
                        }
                    } else {
                        ui.label("Loading conversation…");
                    }
                    if scroll_bottom {
                        ui.scroll_to_cursor(Some(Align::BOTTOM));
                    }
                });
        });
    if let Some((guid, name)) = download {
        app.download(ctx, guid, name);
    }
}

fn composer(app: &mut App, ctx: &egui::Context, guid: &str) {
    let colors = palette(app.dark);
    egui::TopBottomPanel::bottom("composer")
        .frame(panel(colors.background, 18))
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(colors.surface)
                .stroke(egui::Stroke::new(1.0_f32, colors.border))
                .corner_radius(16)
                .inner_margin(14)
                .show(ui, |ui| {
                    let draft = app.drafts.entry(guid.to_owned()).or_default();
                    let input_id = egui::Id::new(("message_composer", guid));
                    let send_shortcut = crate::composer::take_send_key(ui, input_id);
                    let mut output = TextEdit::multiline(draft)
                        .id(input_id)
                        .return_key(egui::KeyboardShortcut::new(
                            egui::Modifiers::SHIFT,
                            egui::Key::Enter,
                        ))
                        .hint_text("Write a message…")
                        .frame(false)
                        .desired_rows(2)
                        .desired_width(f32::INFINITY)
                        .show(ui);
                    let can_send = !app.busy && !draft.trim().is_empty();
                    if output.response.changed() {
                        app.save_draft(guid);
                    }
                    let mut send = send_shortcut && can_send;
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(!app.busy, egui::Button::new("+ Attach").frame(false))
                            .clicked()
                        {
                            app.attach(ctx);
                        }
                        if let Some(emoji) = crate::composer::emoji_picker(ui) {
                            let draft = app.drafts.entry(guid.to_owned()).or_default();
                            crate::composer::insert_emoji(draft, &mut output.state, emoji);
                            output.state.clone().store(ctx, input_id);
                            output.response.request_focus();
                            app.save_draft(guid);
                        }
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            send |= ui
                                .add_enabled(
                                    can_send,
                                    egui::Button::new(RichText::new("Send").color(Color32::WHITE))
                                        .fill(BLUE)
                                        .min_size(egui::vec2(80.0, 34.0)),
                                )
                                .clicked();
                            if ui.available_width() > 160.0 {
                                ui.label(
                                    RichText::new("Enter to send · Shift+Enter for a new line")
                                        .size(11.0)
                                        .color(colors.muted),
                                );
                            }
                        });
                    });
                    if send {
                        output.response.request_focus();
                        app.send(ctx);
                    }
                });
        });
}

fn message_bubble(
    ui: &mut egui::Ui,
    message: &Message,
    sender: Option<&str>,
    busy: bool,
    download: &mut Option<(String, String)>,
) {
    let colors = palette(ui.visuals().dark_mode);
    let outgoing = message.is_from_me;
    let align = if outgoing { Align::RIGHT } else { Align::LEFT };
    let width = (ui.available_width() * 0.78).min(620.0);
    ui.with_layout(Layout::top_down(align), |ui| {
        ui.spacing_mut().item_spacing.y = 5.0;
        if let Some(sender) = sender {
            ui.label(RichText::new(sender).size(11.0).color(colors.muted));
        }
        let fill = if outgoing { BLUE } else { colors.surface };
        egui::Frame::new()
            .fill(fill)
            .stroke(if outgoing {
                egui::Stroke::NONE
            } else {
                egui::Stroke::new(1.0_f32, colors.border)
            })
            .corner_radius(18)
            .inner_margin(egui::Margin::symmetric(16, 12))
            .show(ui, |ui| {
                ui.set_max_width(width);
                if outgoing {
                    ui.visuals_mut().override_text_color = Some(Color32::WHITE);
                }
                if let Some(subject) = message.subject.as_ref().filter(|s| !s.is_empty()) {
                    ui.strong(subject);
                }
                if let Some(text) = message.text.as_ref().filter(|s| !s.is_empty()) {
                    ui.add(egui::Label::new(text).wrap().selectable(true));
                } else if message.attachments.is_empty() {
                    ui.label("[Non-text message]");
                }
                for attachment in &message.attachments {
                    let name = attachment
                        .transfer_name
                        .clone()
                        .unwrap_or_else(|| "attachment".into());
                    let label = if let Some(size) = attachment.total_bytes {
                        format!("Save {name} · {:.1} KB", size as f64 / 1024.0)
                    } else {
                        format!("Save {name}")
                    };
                    if ui.add_enabled(!busy, egui::Button::new(label)).clicked() {
                        *download = Some((attachment.guid.clone(), name));
                    }
                }
            });
        let date = message
            .date_created
            .and_then(|ms| Local.timestamp_millis_opt(ms).single())
            .map(|d| d.format("%-I:%M %p").to_string())
            .unwrap_or_default();
        ui.label(
            RichText::new(if outgoing {
                format!("{date}  ·  {}", message.delivery())
            } else {
                date
            })
            .size(10.5)
            .color(colors.muted),
        );
        ui.add_space(8.0);
    });
}

fn new_chat(app: &mut App, ctx: &egui::Context) {
    let mut open = true;
    egui::Window::new("New iMessage")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(440.0)
        .show(ctx, |ui| {
            ui.add_enabled_ui(!app.busy, |ui| {
                ui.label("Phone numbers or email addresses, separated by commas");
                ui.add(TextEdit::singleline(&mut app.recipients).desired_width(f32::INFINITY));
                ui.label("First message");
                let input_id = egui::Id::new("new_chat_message");
                let send_shortcut = crate::composer::take_send_key(ui, input_id);
                let mut output = TextEdit::multiline(&mut app.initial_message)
                    .id(input_id)
                    .return_key(egui::KeyboardShortcut::new(
                        egui::Modifiers::SHIFT,
                        egui::Key::Enter,
                    ))
                    .desired_rows(4)
                    .desired_width(f32::INFINITY)
                    .show(ui);
                if let Some(emoji) = crate::composer::emoji_picker(ui) {
                    crate::composer::insert_emoji(
                        &mut app.initial_message,
                        &mut output.state,
                        emoji,
                    );
                    output.state.store(ctx, input_id);
                    output.response.request_focus();
                }
                ui.small("Enter to send · Shift+Enter for a new line");
                let valid = app.recipients.split(',').any(|s| !s.trim().is_empty())
                    && !app.initial_message.trim().is_empty();
                if ui
                    .add_enabled(valid, egui::Button::new("Create and send"))
                    .clicked()
                    || (valid && send_shortcut)
                {
                    app.create_chat(ctx);
                }
            });
        });
    if !open {
        app.new_chat = false;
    }
}
