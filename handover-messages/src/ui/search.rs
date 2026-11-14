use super::*;

fn matches(query: &str, fields: &[String]) -> bool {
    query.split_whitespace().all(|term| {
        let term = term.to_lowercase();
        let digits = term
            .chars()
            .filter(char::is_ascii_digit)
            .collect::<String>();
        fields.iter().any(|field| {
            field.to_lowercase().contains(&term)
                || (digits.len() >= 3
                    && term
                        .chars()
                        .all(|c| c.is_ascii_digit() || c.is_whitespace() || "+()- .".contains(c))
                    && field
                        .chars()
                        .filter(char::is_ascii_digit)
                        .collect::<String>()
                        .contains(&digits))
        })
    })
}

impl Window {
    pub(super) fn conversation_matches(&self, conversation: &Conversation, query: &str) -> bool {
        let mut fields = vec![conversation.title.clone().unwrap_or_default()];
        for participant in &conversation.participants {
            fields.push(self.photos.borrow().label(participant));
            if let Some(address) = &participant.address {
                fields.push(address.clone());
            }
        }
        matches(query, &fields)
    }
    pub(super) fn matching_contacts(&self, contacts: &[Contact], query: &str) -> Vec<Contact> {
        let mut seen = BTreeSet::new();
        contacts
            .iter()
            .filter(|contact| {
                let mut fields = vec![contact.display_name.clone()];
                fields.extend(contact.phones.iter().chain(&contact.emails).cloned());
                let identity = (
                    if contact.phones.is_empty() && contact.emails.is_empty() {
                        format!(
                            "{}:{}:{}",
                            contact.display_name.to_lowercase(),
                            contact.device_id,
                            contact.local_id
                        )
                    } else {
                        contact.display_name.to_lowercase()
                    },
                    contact
                        .phones
                        .iter()
                        .chain(&contact.emails)
                        .filter_map(|address| crate::contact_photos::key(address))
                        .collect::<BTreeSet<_>>(),
                );
                matches(query, &fields) && seen.insert(identity)
            })
            .take(30)
            .cloned()
            .collect()
    }
    pub(super) fn sidebar_heading(&self, title: &str) {
        let label = gtk::Label::new(Some(title));
        label.add_css_class("search-section");
        label.set_xalign(0.0);
        let row = gtk::ListBoxRow::new();
        row.set_selectable(false);
        row.set_activatable(false);
        row.set_child(Some(&label));
        self.list.append(&row);
    }
    pub(super) fn show_contact(self: &Rc<Self>, contact: &Contact) {
        let window = gtk::Window::builder()
            .application(&self.window.application().unwrap())
            .transient_for(&self.window)
            .title("Contact details")
            .default_width(420)
            .build();
        let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
        content.set_margin_top(24);
        content.set_margin_bottom(24);
        content.set_margin_start(24);
        content.set_margin_end(24);
        let title = gtk::Label::new(Some(&contact.display_name));
        title.add_css_class("title-2");
        title.set_wrap(true);
        content.append(&title);
        for address in contact.phones.iter().chain(&contact.emails) {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            let label = gtk::Label::new(Some(address));
            label.set_selectable(true);
            label.set_xalign(0.0);
            label.set_hexpand(true);
            label.set_wrap(true);
            let copy = gtk::Button::with_label("Copy");
            let address = address.clone();
            let weak = window.downgrade();
            copy.connect_clicked(move |_| {
                if let Some(window) = weak.upgrade() {
                    window.clipboard().set_text(&address);
                }
            });
            row.append(&label);
            row.append(&copy);
            content.append(&row);
        }
        let addresses = contact
            .phones
            .iter()
            .chain(&contact.emails)
            .filter_map(|address| crate::contact_photos::key(address))
            .collect::<BTreeSet<_>>();
        let model = self.model.borrow();
        let chats = model
            .conversations
            .iter()
            .filter(|conversation| {
                self.filter
                    .borrow()
                    .as_ref()
                    .is_none_or(|account| &conversation.id.account_id == account)
                    && conversation
                        .participants
                        .iter()
                        .filter(|participant| !participant.is_self)
                        .filter_map(|participant| participant.address.as_deref())
                        .filter_map(crate::contact_photos::key)
                        .any(|address| addresses.contains(&address))
            })
            .collect::<Vec<_>>();
        if chats.is_empty() {
            let note = gtk::Label::new(Some(
                if contact.phones.is_empty() && contact.emails.is_empty() {
                    "No saved phone number or email address."
                } else if self.filter.borrow().is_some() {
                    "No existing conversation in this account. You can copy a contact address."
                } else {
                    "No existing conversation. You can copy a contact address."
                },
            ));
            note.set_wrap(true);
            note.add_css_class("dim-label");
            content.append(&note);
        }
        for chat in chats.into_iter().take(20) {
            let name = self.conversation_label(&model, &chat.id);
            let account = model
                .accounts
                .iter()
                .find(|account| account.id == chat.id.account_id)
                .map(|account| account.label.as_str())
                .unwrap_or("Messages");
            let open = gtk::Button::with_label(&format!("Open {name} · {account}"));
            let weak = Rc::downgrade(self);
            let popup = window.downgrade();
            let id = chat.id.clone();
            open.connect_clicked(move |_| {
                if let Some(this) = weak.upgrade() {
                    this.search.set_text("");
                    this.search_changed();
                    this.open(id.clone(), None);
                }
                if let Some(window) = popup.upgrade() {
                    window.close();
                }
            });
            content.append(&open);
        }
        window.set_child(Some(&content));
        let keys = gtk::EventControllerKey::new();
        let weak = window.downgrade();
        keys.connect_key_pressed(move |_, key, _, _| {
            if key == gtk::gdk::Key::Escape {
                if let Some(window) = weak.upgrade() {
                    window.close();
                }
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        window.add_controller(keys);
        window.present();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn contact_query_matches_formatted_phone_and_requires_all_terms() {
        let fields = vec!["Maya Chen".into(), "+1 (415) 555-0123".into()];
        assert!(matches("maya 415555", &fields));
        assert!(matches("CHEN", &fields));
        assert!(!matches("Maya Jordan", &fields));
    }
}
