use crate::{
    app::App,
    private_api::{self, Dialog},
};
use bluebubbles_linux::{
    api_actions::Action,
    model::{AssociatedMessageType, Message},
};
use eframe::egui::{self, RichText, TextEdit};

pub enum Intent {
    Reply(Box<Message>),
    Dialog(Dialog),
    Action(Action),
}

pub fn toolbar(ui: &mut egui::Ui, app: &mut App, ctx: &egui::Context) {
    if !app.connected {
        return;
    }
    ui.menu_button(
        if app.private_available() {
            "Private API · on"
        } else {
            "Private API"
        },
        |ui| {
            if let Some(info) = &app.server_info {
                ui.label(format!(
                    "Server {} · macOS {}",
                    info.server_version, info.os_version
                ));
                ui.label(if info.private_available() {
                    "Helper connected"
                } else {
                    "Enable Private API and connect its helper on your Mac."
                });
            } else {
                ui.label("Checking server capabilities…");
            }
            if ui
                .checkbox(&mut app.private_enabled, "Use Private API when available")
                .changed()
                && !app.private_enabled
            {
                app.stop_typing();
            }
            if ui
                .checkbox(&mut app.send_typing, "Send typing indicators")
                .changed()
                && !app.send_typing
            {
                app.stop_typing();
            }
            if ui
                .add_enabled(!app.busy, egui::Button::new("Check server capabilities"))
                .clicked()
            {
                app.refresh_capabilities(ctx);
            }
            ui.hyperlink_to(
                "Private API setup",
                "https://docs.bluebubbles.app/private-api/",
            );
        },
    );
}

pub fn conversation_menu(ui: &mut egui::Ui, app: &mut App, ctx: &egui::Context, guid: &str) {
    if !app.private_available() {
        return;
    }
    ui.menu_button("Conversation actions", |ui| {
        if ui
            .add_enabled(!app.busy, egui::Button::new("Mark read on Mac"))
            .clicked()
        {
            app.private_action(
                ctx,
                Action::Read {
                    chat: guid.into(),
                    read: true,
                },
            );
            ui.close_menu();
        }
        if ui
            .add_enabled(
                !app.busy
                    && app
                        .server_info
                        .as_ref()
                        .is_some_and(|i| i.macos_at_least(13)),
                egui::Button::new("Mark unread on Mac"),
            )
            .clicked()
        {
            app.private_action(
                ctx,
                Action::Read {
                    chat: guid.into(),
                    read: false,
                },
            );
            ui.close_menu();
        }
        if private_api::group(guid)
            && ui
                .add_enabled(!app.busy, egui::Button::new("Manage group…"))
                .clicked()
        {
            let name = app
                .chats
                .iter()
                .find(|c| c.guid == guid)
                .and_then(|c| c.display_name.clone())
                .unwrap_or_default();
            app.private_dialog = Some(Dialog::Group {
                chat: guid.into(),
                name,
                address: String::new(),
            });
            ui.close_menu();
        }
    });
}

pub fn composer_options(ui: &mut egui::Ui, app: &mut App, guid: &str) {
    let enabled = app.private_for(guid);
    if !enabled && app.extras.get(guid).is_none_or(|extras| extras.is_empty()) {
        return;
    }
    let extras = app.extras.entry(guid.into()).or_default();
    if let Some((_, text)) = extras.reply.clone() {
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(format!(
                    "Replying to: {}",
                    text.chars().take(80).collect::<String>()
                ))
                .small(),
            );
            if ui.small_button("Cancel reply").clicked() {
                extras.reply = None;
            }
        });
    }
    ui.horizontal(|ui| {
        ui.menu_button("Subject / effect", |ui| {
            ui.add_enabled_ui(enabled, |ui| {
                ui.add(TextEdit::singleline(&mut extras.subject).hint_text("Subject (optional)"));
                egui::ComboBox::from_id_salt(("send_effect", guid))
                    .selected_text(
                        private_api::EFFECTS
                            .iter()
                            .find(|(_, id)| *id == extras.effect)
                            .map(|(name, _)| *name)
                            .unwrap_or("None"),
                    )
                    .show_ui(ui, |ui| {
                        for (name, id) in private_api::EFFECTS {
                            ui.selectable_value(&mut extras.effect, (*id).into(), *name);
                        }
                    });
            });
            if ui.button("Clear sending options").clicked() {
                *extras = Default::default();
            }
        });
        if !extras.subject.is_empty() {
            ui.small(format!("Subject: {}", extras.subject));
        }
        if !extras.effect.is_empty() {
            ui.small("Effect selected");
        }
        if !enabled {
            ui.colored_label(
                ui.visuals().error_fg_color,
                "Private API unavailable — sending options kept",
            );
        }
    });
}

pub fn message_menu(ui: &mut egui::Ui, app: &App, guid: &str, message: &Message) -> Option<Intent> {
    let mut intent = None;
    if !app.private_for(guid) {
        return None;
    }
    ui.push_id((&message.guid, "private_actions"), |ui| {
        ui.menu_button("Actions", |ui| {
            if let Some(text) = &message.text { if ui.button("Copy text").clicked() { ui.ctx().copy_text(text.clone()); ui.close_menu(); } }
            // Until individual message parts are selectable, only target part zero
            // of a simple text message or a single standalone attachment.
            let simple = message.associated_message_guid.is_none() && (message.attachments.is_empty() || (message.attachments.len()==1 && message.text.as_ref().is_none_or(|t| t.is_empty()))) && message.extra.get("dateRetracted").and_then(|v| v.as_i64()).unwrap_or(0)==0;
            ui.add_enabled_ui(!app.busy && simple, |ui| {
                if ui.add_enabled(app.server_info.as_ref().is_some_and(|i| i.macos_at_least(11)), egui::Button::new("Reply")).clicked() { intent=Some(Intent::Reply(Box::new(message.clone()))); ui.close_menu(); }
                ui.menu_button("React", |ui| {
                    for (label, reaction) in private_api::REACTIONS {
                        if ui.button(*label).clicked() {
                            intent=Some(Intent::Action(Action::React { chat:guid.into(), message:message.guid.clone(), text:message.text.clone().unwrap_or_default(), reaction:(*reaction).into() })); ui.close_menu();
                        }
                    }
                    ui.menu_button("Remove my reaction", |ui| { for (label,reaction) in private_api::REACTIONS {
                        if ui.button(*label).clicked() { intent=Some(Intent::Action(Action::React { chat:guid.into(), message:message.guid.clone(), text:message.text.clone().unwrap_or_default(), reaction:format!("-{reaction}") })); ui.close_menu(); }
                    }});
                });
                let now = chrono::Utc::now().timestamp_millis();
                if message.is_from_me {
                    if ui.add_enabled(private_api::editable(app.server_info.as_ref(),guid,message,false,now),egui::Button::new("Edit…")).on_hover_text("Your text messages, within 15 minutes; macOS 13+ and server 1.2.6+").clicked() {
                        intent=Some(Intent::Dialog(Dialog::Edit {chat:guid.into(),message:Box::new(message.clone()),text:message.text.clone().unwrap_or_default()})); ui.close_menu();
                    }
                    if ui.add_enabled(private_api::editable(app.server_info.as_ref(),guid,message,true,now),egui::Button::new("Undo send…")).on_hover_text("Your messages, within 2 minutes; macOS 13+ and server 1.2.6+").clicked() {
                        intent=Some(Intent::Dialog(Dialog::Confirm {action:Action::Unsend{message:message.guid.clone()}, description:"Undo sending this message? Recipients on older software may still see it.".into()})); ui.close_menu();
                    }
                }
            });
            if !simple { ui.small("Actions on individual parts of this message are not supported yet."); }
        });
    });
    intent
}

pub fn apply(app: &mut App, ctx: &egui::Context, guid: &str, intent: Intent) {
    match intent {
        Intent::Reply(message) => {
            let preview = message.preview();
            app.extras.entry(guid.into()).or_default().reply = Some((message.guid, preview));
        }
        Intent::Dialog(dialog) => app.private_dialog = Some(dialog),
        Intent::Action(action) => app.private_action(ctx, action),
    }
}

pub fn dialog(app: &mut App, ctx: &egui::Context) {
    let Some(mut dialog) = app.private_dialog.take() else {
        return;
    };
    let mut open = true;
    let mut action = None;
    let mut next = None;
    egui::Window::new("Private API action").id(egui::Id::new("private_action_dialog")).open(&mut open).collapsible(false).default_width(440.0).show(ctx, |ui| {
        ui.add_enabled_ui(!app.busy, |ui| match &mut dialog {
            Dialog::Edit { chat, message, text } => {
                ui.label("Edit message");
                ui.add(TextEdit::multiline(text).desired_width(420.0).desired_rows(4));
                if ui.add_enabled(!text.trim().is_empty() && private_api::editable(app.server_info.as_ref(),chat,message,false,chrono::Utc::now().timestamp_millis()),egui::Button::new("Save edit")).clicked() {
                    action=Some(Action::Edit { message:message.guid.clone(),text:text.clone(),original:format!("Edited to “{text}”") });
                }
            }
            Dialog::Confirm { action: requested, description } => {
                ui.label(description.as_str());
                if ui.button("Confirm").clicked() { action=Some(requested.clone()); }
            }
            Dialog::Group { chat, name, address } => {
                ui.label("Group name"); ui.text_edit_singleline(name);
                if ui.add_enabled(!name.trim().is_empty(),egui::Button::new("Rename group")).clicked() { action=Some(Action::Rename{chat:chat.clone(),name:name.trim().into()}); }
                ui.separator();
                ui.label("Add a phone number or email address"); ui.text_edit_singleline(address);
                if ui.add_enabled(!address.trim().is_empty(),egui::Button::new("Add participant")).clicked() { action=Some(Action::Participant{chat:chat.clone(),address:address.trim().into(),add:true}); }
                if let Some(group)=app.chats.iter().find(|c| &c.guid==chat) {
                    for participant in &group.participants {
                        ui.horizontal(|ui| { ui.label(&participant.address); if ui.small_button("Remove…").clicked() { next=Some(Dialog::Confirm{action:Action::Participant{chat:chat.clone(),address:participant.address.clone(),add:false},description:format!("Remove {} from the group?",participant.address)}); } });
                    }
                }
                if ui.button("Leave group…").clicked() { next=Some(Dialog::Confirm{action:Action::Leave{chat:chat.clone()},description:"Leave this group conversation? You may need another participant to add you again.".into()}); }
            }
        });
    });
    if open {
        app.private_dialog = Some(next.unwrap_or(dialog));
    }
    if let Some(action) = action {
        app.private_action(ctx, action);
    }
}

pub fn decorations(ui: &mut egui::Ui, message: &Message, history: &[Message]) {
    if let Some(origin) = &message.thread_originator_guid {
        let target = origin.rsplit('/').next().unwrap_or(origin);
        let preview = history
            .iter()
            .find(|m| m.guid == target)
            .map(Message::preview)
            .unwrap_or_else(|| "Earlier message".into());
        ui.small(format!(
            "↳ Reply to: {}",
            preview.chars().take(80).collect::<String>()
        ));
    }
    let mut reactions = std::collections::BTreeMap::new();
    for reaction in history.iter().filter(|m| {
        m.associated_message_guid
            .as_ref()
            .is_some_and(|guid| guid.rsplit('/').next() == Some(message.guid.as_str()))
    }) {
        let kind = match &reaction.associated_message_type {
            Some(AssociatedMessageType::Name(name)) => name.clone(),
            Some(AssociatedMessageType::Code(code)) => {
                let index = code % 1000;
                if !(0..6).contains(&index) {
                    continue;
                }
                format!(
                    "{}{}",
                    if *code >= 3000 { "-" } else { "" },
                    private_api::REACTIONS[index as usize].1
                )
            }
            None => continue,
        };
        let sender = if reaction.is_from_me {
            "You".to_owned()
        } else {
            reaction
                .handle
                .as_ref()
                .map(|h| h.address.clone())
                .unwrap_or_default()
        };
        if kind.starts_with('-') {
            reactions.remove(&sender);
        } else {
            reactions.insert(sender, kind);
        }
    }
    if !reactions.is_empty() {
        ui.label(
            RichText::new(
                reactions
                    .into_iter()
                    .map(|(sender, kind)| format!("{sender}: {kind}"))
                    .collect::<Vec<_>>()
                    .join(" · "),
            )
            .small(),
        );
    }
}
