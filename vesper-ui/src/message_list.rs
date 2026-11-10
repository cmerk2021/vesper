//! The message view: a `gtk::ListView` over a `gio::ListStore` of
//! `MessageObject`s, with consecutive-message grouping and auto-scroll.

use std::cell::RefCell;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, gio, glib};
use vesper_core::models::Message;

use crate::avatar_cache;

/// Messages by the same author within this window are visually grouped.
const GROUP_WINDOW_SECONDS: i64 = 7 * 60;
const AVATAR_SIZE: i32 = 40;

#[derive(Debug, Default, Clone)]
pub struct MessageData {
    pub id: u64,
    pub author_id: u64,
    pub author_name: String,
    pub avatar_url: Option<String>,
    pub timestamp_unix: i64,
    pub timestamp_display: String,
    pub content: String,
    pub grouped: bool,
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct MessageObject {
        pub data: RefCell<MessageData>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MessageObject {
        const NAME: &'static str = "VesperMessageObject";
        type Type = super::MessageObject;
    }

    impl ObjectImpl for MessageObject {}
}

glib::wrapper! {
    pub struct MessageObject(ObjectSubclass<imp::MessageObject>);
}

impl MessageObject {
    pub fn new(data: MessageData) -> Self {
        let obj: Self = glib::Object::new();
        obj.imp().data.replace(data);
        obj
    }

    pub fn data(&self) -> MessageData {
        self.imp().data.borrow().clone()
    }
}

fn parse_timestamp(ts: Option<&str>) -> (i64, String) {
    let Some(ts) = ts else {
        return (0, String::new());
    };
    match glib::DateTime::from_iso8601(ts, None) {
        Ok(dt) => {
            let local = dt.to_local().unwrap_or(dt);
            let display = local
                .format("%H:%M")
                .map(|s| s.to_string())
                .unwrap_or_default();
            (local.to_unix(), display)
        }
        Err(_) => (0, String::new()),
    }
}

/// Converts a core message into display data, computing the grouping flag
/// against the previous message in the list.
pub fn message_data(msg: &Message, previous: Option<&MessageData>) -> MessageData {
    let author = msg.author.clone().unwrap_or_default();
    let author_id = author.id.map(|s| s.0).unwrap_or(0);
    let (timestamp_unix, timestamp_display) = parse_timestamp(msg.timestamp.as_deref());
    let grouped = previous.is_some_and(|prev| {
        prev.author_id == author_id
            && author_id != 0
            && timestamp_unix - prev.timestamp_unix >= 0
            && timestamp_unix - prev.timestamp_unix < GROUP_WINDOW_SECONDS
    });
    MessageData {
        id: msg.id.0,
        author_id,
        author_name: author.display_name(),
        avatar_url: author.avatar_url(80),
        timestamp_unix,
        timestamp_display,
        content: msg.content.clone().unwrap_or_default(),
        grouped,
    }
}

fn build_row() -> gtk::Box {
    let root = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    root.add_css_class("vesper-message");

    let avatar_slot = gtk::Box::new(gtk::Orientation::Vertical, 0);
    avatar_slot.set_valign(gtk::Align::Start);
    avatar_slot.set_size_request(AVATAR_SIZE + 12, -1);

    let avatar = gtk::Picture::new();
    avatar.add_css_class("vesper-avatar");
    avatar.set_size_request(AVATAR_SIZE, AVATAR_SIZE);
    avatar.set_content_fit(gtk::ContentFit::Cover);
    // VERIFY: circular clipping (Overflow::Hidden + border-radius) needs a
    // visual check on a real display.
    avatar.set_overflow(gtk::Overflow::Hidden);
    avatar_slot.append(&avatar);
    root.append(&avatar_slot);

    let body = gtk::Box::new(gtk::Orientation::Vertical, 2);
    body.set_hexpand(true);

    let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let author = gtk::Label::new(None);
    author.add_css_class("vesper-author");
    author.set_xalign(0.0);
    let timestamp = gtk::Label::new(None);
    timestamp.add_css_class("vesper-timestamp");
    timestamp.set_xalign(0.0);
    timestamp.set_valign(gtk::Align::Baseline);
    header.append(&author);
    header.append(&timestamp);
    body.append(&header);

    let content = gtk::Label::new(None);
    content.add_css_class("vesper-content");
    content.set_xalign(0.0);
    content.set_wrap(true);
    content.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    content.set_selectable(true);
    body.append(&content);

    root.append(&body);
    root
}

/// Builds the ListView and its factory over `store`.
pub fn build_message_view(store: &gio::ListStore) -> gtk::ListView {
    let factory = gtk::SignalListItemFactory::new();

    factory.connect_setup(|_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().unwrap();
        item.set_child(Some(&build_row()));
        item.set_activatable(false);
        item.set_selectable(false);
    });

    factory.connect_bind(|_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().unwrap();
        let obj = item.item().and_downcast::<MessageObject>().unwrap();
        let data = obj.data();
        let root = item.child().and_downcast::<gtk::Box>().unwrap();

        // Recover the widget tree built in setup.
        let avatar_slot = root.first_child().and_downcast::<gtk::Box>().unwrap();
        let avatar = avatar_slot
            .first_child()
            .and_downcast::<gtk::Picture>()
            .unwrap();
        let body = avatar_slot
            .next_sibling()
            .and_downcast::<gtk::Box>()
            .unwrap();
        let header = body.first_child().and_downcast::<gtk::Box>().unwrap();
        let author = header.first_child().and_downcast::<gtk::Label>().unwrap();
        let timestamp = author.next_sibling().and_downcast::<gtk::Label>().unwrap();
        let content = header.next_sibling().and_downcast::<gtk::Label>().unwrap();

        author.set_label(&data.author_name);
        timestamp.set_label(&data.timestamp_display);
        content.set_label(&data.content);

        if data.grouped {
            root.add_css_class("vesper-message-grouped");
            header.set_visible(false);
            avatar.set_visible(false);
        } else {
            root.remove_css_class("vesper-message-grouped");
            header.set_visible(true);
            avatar.set_visible(true);
            if let Some(url) = &data.avatar_url {
                avatar_cache::load_into(&avatar, url.clone());
            } else {
                avatar.set_widget_name("");
                avatar.set_paintable(gdk::Paintable::NONE);
            }
        }
    });

    let selection = gtk::NoSelection::new(Some(store.clone()));
    let view = gtk::ListView::new(Some(selection), Some(factory));
    view.add_css_class("vesper-message-list");
    view.set_vexpand(true);
    view
}

/// Last message data in the store, for grouping of live appends.
pub fn last_message_data(store: &gio::ListStore) -> Option<MessageData> {
    let n = store.n_items();
    if n == 0 {
        return None;
    }
    store
        .item(n - 1)
        .and_downcast::<MessageObject>()
        .map(|o| o.data())
}
