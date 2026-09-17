use crate::app::App;
use bluebubbles_linux::{
    api::ApiResult,
    api_actions::{contact_names, normalize_address},
};
use eframe::egui;
use serde_json::Value;
use std::sync::mpsc::{self, Receiver};

pub struct Editor {
    address: String,
    name: String,
    contact: Option<Value>,
    rx: Option<Receiver<ApiResult<Option<Value>>>>,
    saving: bool,
    ready: bool,
    error: Option<String>,
}
impl App {
    pub fn contact_name(&self, address: &str) -> String {
        self.contacts
            .get(&normalize_address(address))
            .cloned()
            .unwrap_or_else(|| address.to_owned())
    }
    pub fn edit_contact(&mut self, address: String) {
        let Some(api) = self.api.clone() else {
            return;
        };
        let name = self.contact_name(&address);
        let name = if name == address { String::new() } else { name };
        let lookup = address.clone();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(api.editable_contact(&lookup));
        });
        self.contact_editor = Some(Editor {
            address,
            name,
            contact: None,
            rx: Some(rx),
            saving: false,
            ready: false,
            error: None,
        });
    }
}

pub fn show(app: &mut App, ctx: &egui::Context) {
    let Some(mut editor) = app.contact_editor.take() else {
        return;
    };
    let mut open = true;
    let mut save = false;
    if let Some(rx) = &editor.rx {
        match rx.try_recv() {
            Ok(Ok(contact)) => {
                if editor.saving {
                    if let Some(contact) = contact {
                        app.contacts.extend(contact_names(&[contact]));
                    }
                    app.status = "Contact saved on the BlueBubbles server".into();
                    return;
                }
                editor.name = contact
                    .as_ref()
                    .map(|c| {
                        contact_names(std::slice::from_ref(c))
                            .values()
                            .next()
                            .cloned()
                            .unwrap_or_default()
                    })
                    .unwrap_or_default();
                editor.contact = contact;
                editor.ready = true;
                editor.rx = None;
            }
            Ok(Err(error)) => {
                editor.error = Some(error);
                editor.rx = None;
                editor.saving = false;
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                editor.error = Some("Contact request stopped. Close and reopen the editor.".into());
                editor.rx = None;
                editor.saving = false;
            }
            Err(mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
            }
        }
    }
    egui::Window::new("Edit server contact").open(&mut open).collapsible(false).resizable(false).default_width(440.0).show(ctx,|ui| {
        ui.label(&editor.address);
        ui.label("Display name");
        ui.add_enabled_ui(editor.ready && !editor.saving,|ui| {
            let input=ui.add(egui::TextEdit::singleline(&mut editor.name).desired_width(420.0).char_limit(200));
            save=input.lost_focus() && ui.input(|i|i.key_pressed(egui::Key::Enter)) && !editor.name.trim().is_empty();
            ui.label("Saved to BlueBubbles Server’s contacts. Phone numbers, email addresses, and the conversation’s recipient stay unchanged.");
            save|=ui.add_enabled(!editor.name.trim().is_empty(),egui::Button::new("Save to server")).clicked();
        });
        if editor.rx.is_some() {ui.horizontal(|ui|{ui.spinner();ui.label(if editor.saving {"Saving contact…"} else {"Checking server contact support…"});});}
        if let Some(error)=&editor.error {ui.label(egui::RichText::new(error).color(ui.visuals().error_fg_color));}
    });
    if save {
        if let Some(api) = app.api.clone() {
            let address = editor.address.clone();
            let name = editor.name.clone();
            let contact = editor.contact.clone();
            let (tx, rx) = mpsc::channel();
            std::thread::spawn(move || {
                let _ = tx.send(
                    api.save_contact_name(contact.as_ref(), &address, &name)
                        .map(Some),
                );
            });
            editor.rx = Some(rx);
            editor.saving = true;
            editor.error = None;
        }
    }
    // Closing during a save retains the completion so the contact list is updated.
    if open || editor.saving {
        app.contact_editor = Some(editor);
    }
}
