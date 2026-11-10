//! The main app shell: server rail, channel list, message area, input.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gdk, gio, glib};
use tracing::debug;
use vesper_core::models::{Channel, ChannelType, Guild, Message, Snowflake, User};
use vesper_core::{Command, CommandSender};

use crate::avatar_cache;
use crate::message_list::{self, MessageObject};

/// Sidebar selection: the DM list or a guild.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RailSelection {
    Dms,
    Guild(Snowflake),
}

struct State {
    guilds: Vec<Guild>,
    private_channels: Vec<Channel>,
    users: HashMap<Snowflake, User>,
    selection: RailSelection,
    current_channel: Option<Snowflake>,
}

pub struct Shell {
    pub root: gtk::Box,
    cmd: CommandSender,
    state: RefCell<State>,
    server_list: gtk::Box,
    sidebar_title: gtk::Label,
    channel_list: gtk::ListBox,
    channel_title: gtk::Label,
    message_store: gio::ListStore,
    message_scroll: gtk::ScrolledWindow,
    input: gtk::TextView,
}

impl Shell {
    pub fn new(cmd: CommandSender) -> Rc<Self> {
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        root.add_css_class("vesper-root");

        // ---- Server rail ----
        let server_list = gtk::Box::new(gtk::Orientation::Vertical, 0);
        server_list.add_css_class("vesper-server-rail");
        let rail_scroll = gtk::ScrolledWindow::new();
        rail_scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::External);
        rail_scroll.set_child(Some(&server_list));
        rail_scroll.set_width_request(64);
        root.append(&rail_scroll);

        // ---- Sidebar: channel list ----
        let sidebar = gtk::Box::new(gtk::Orientation::Vertical, 0);
        sidebar.add_css_class("vesper-sidebar");
        sidebar.set_width_request(230);
        let sidebar_title = gtk::Label::new(Some("Direct Messages"));
        sidebar_title.add_css_class("vesper-sidebar-title");
        sidebar_title.set_xalign(0.0);
        sidebar.append(&sidebar_title);

        let channel_list = gtk::ListBox::new();
        channel_list.add_css_class("vesper-channel-list");
        channel_list.set_selection_mode(gtk::SelectionMode::Single);
        let channel_scroll = gtk::ScrolledWindow::new();
        channel_scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        channel_scroll.set_vexpand(true);
        channel_scroll.set_child(Some(&channel_list));
        sidebar.append(&channel_scroll);
        root.append(&sidebar);

        // ---- Main area: messages + input ----
        // The layout keeps the member list for a later pass in mind: the main
        // column is a horizontal box with room for a trailing panel.
        let main_area = gtk::Box::new(gtk::Orientation::Vertical, 0);
        main_area.add_css_class("vesper-main");
        main_area.set_hexpand(true);

        let channel_title = gtk::Label::new(Some("Select a channel"));
        channel_title.add_css_class("vesper-sidebar-title");
        channel_title.set_xalign(0.0);
        main_area.append(&channel_title);

        let message_store = gio::ListStore::new::<MessageObject>();
        let message_view = message_list::build_message_view(&message_store);
        let message_scroll = gtk::ScrolledWindow::new();
        message_scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        message_scroll.set_vexpand(true);
        message_scroll.set_child(Some(&message_view));
        main_area.append(&message_scroll);

        let input_area = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        input_area.add_css_class("vesper-input-area");
        let input = gtk::TextView::new();
        input.add_css_class("vesper-input");
        input.set_wrap_mode(gtk::WrapMode::WordChar);
        input.set_hexpand(true);
        input.set_accepts_tab(false);
        let input_scroll = gtk::ScrolledWindow::new();
        input_scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        input_scroll.set_max_content_height(160);
        input_scroll.set_propagate_natural_height(true);
        input_scroll.set_hexpand(true);
        input_scroll.set_child(Some(&input));
        input_area.append(&input_scroll);
        main_area.append(&input_area);

        root.append(&main_area);

        let shell = Rc::new(Shell {
            root,
            cmd,
            state: RefCell::new(State {
                guilds: Vec::new(),
                private_channels: Vec::new(),
                users: HashMap::new(),
                selection: RailSelection::Dms,
                current_channel: None,
            }),
            server_list,
            sidebar_title,
            channel_list,
            channel_title,
            message_store,
            message_scroll,
            input,
        });

        // Enter sends; Shift+Enter inserts a newline (TextView default).
        {
            let shell_weak = Rc::downgrade(&shell);
            let key = gtk::EventControllerKey::new();
            key.connect_key_pressed(move |_, keyval, _, modifier| {
                let enter = keyval == gdk::Key::Return || keyval == gdk::Key::KP_Enter;
                if enter && !modifier.contains(gdk::ModifierType::SHIFT_MASK) {
                    if let Some(shell) = shell_weak.upgrade() {
                        shell.send_current_input();
                    }
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            });
            shell.input.add_controller(key);
        }

        // Channel selection.
        {
            let shell_weak = Rc::downgrade(&shell);
            shell.channel_list.connect_row_activated(move |_, row| {
                if let Some(shell) = shell_weak.upgrade() {
                    let id = unsafe { row.data::<u64>("channel-id").map(|p| *p.as_ref()) };
                    if let Some(id) = id {
                        shell.open_channel(Snowflake(id));
                    }
                }
            });
        }

        shell
    }

    fn send_current_input(&self) {
        let buffer = self.input.buffer();
        let (start, end) = buffer.bounds();
        let text = buffer.text(&start, &end, false).trim().to_string();
        let Some(channel_id) = self.state.borrow().current_channel else {
            return;
        };
        if text.is_empty() {
            return;
        }
        let _ = self.cmd.send(Command::SendMessage {
            channel_id,
            content: text,
            reply_to: None,
        });
        buffer.set_text("");
    }

    // ---- Data entry points called from the event loop ----

    pub fn on_ready(
        self: &Rc<Self>,
        guilds: Vec<Guild>,
        private_channels: Vec<Channel>,
        users: Vec<User>,
    ) {
        {
            let mut state = self.state.borrow_mut();
            state.users = users
                .into_iter()
                .filter_map(|u| u.id.map(|id| (id, u)))
                .collect();
            state.guilds = guilds;
            state.private_channels = private_channels;
            state.selection = RailSelection::Dms;
            state.current_channel = None;
        }
        self.rebuild_server_rail();
        self.rebuild_channel_list();
    }

    pub fn on_guild_create(self: &Rc<Self>, guild: Guild) {
        {
            let mut state = self.state.borrow_mut();
            if let Some(existing) = state.guilds.iter_mut().find(|g| g.id == guild.id) {
                *existing = guild;
            } else {
                state.guilds.push(guild);
            }
        }
        self.rebuild_server_rail();
    }

    pub fn on_channel_event(self: &Rc<Self>, channel: Channel, deleted: bool) {
        {
            let mut state = self.state.borrow_mut();
            let list: &mut Vec<Channel> = if channel.guild_id.is_some() {
                match state
                    .guilds
                    .iter_mut()
                    .find(|g| Some(g.id) == channel.guild_id)
                {
                    Some(g) => &mut g.channels,
                    None => return,
                }
            } else {
                &mut state.private_channels
            };
            list.retain(|c| c.id != channel.id);
            if !deleted {
                list.push(channel.clone());
            }
        }
        self.rebuild_channel_list();
    }

    pub fn on_guild_channels_loaded(self: &Rc<Self>, guild_id: Snowflake, channels: Vec<Channel>) {
        {
            let mut state = self.state.borrow_mut();
            if let Some(g) = state.guilds.iter_mut().find(|g| g.id == guild_id) {
                g.channels = channels;
            }
        }
        self.rebuild_channel_list();
    }

    pub fn on_messages_loaded(self: &Rc<Self>, channel_id: Snowflake, messages: Vec<Message>) {
        if self.state.borrow().current_channel != Some(channel_id) {
            return;
        }
        self.message_store.remove_all();
        let mut previous: Option<message_list::MessageData> = None;
        for msg in &messages {
            let data = message_list::message_data(msg, previous.as_ref());
            previous = Some(data.clone());
            self.message_store.append(&MessageObject::new(data));
        }
        self.scroll_to_bottom();
    }

    pub fn on_message_create(self: &Rc<Self>, message: &Message) {
        if self.state.borrow().current_channel != Some(message.channel_id) {
            return;
        }
        let was_at_bottom = self.is_at_bottom();
        let previous = message_list::last_message_data(&self.message_store);
        let data = message_list::message_data(message, previous.as_ref());
        self.message_store.append(&MessageObject::new(data));
        if was_at_bottom {
            self.scroll_to_bottom();
        }
    }

    pub fn on_message_delete(self: &Rc<Self>, channel_id: Snowflake, id: Snowflake) {
        if self.state.borrow().current_channel != Some(channel_id) {
            return;
        }
        let n = self.message_store.n_items();
        for i in 0..n {
            if let Some(obj) = self.message_store.item(i).and_downcast::<MessageObject>() {
                if obj.data().id == id.0 {
                    self.message_store.remove(i);
                    break;
                }
            }
        }
    }

    pub fn clear(self: &Rc<Self>) {
        {
            let mut state = self.state.borrow_mut();
            state.guilds.clear();
            state.private_channels.clear();
            state.users.clear();
            state.selection = RailSelection::Dms;
            state.current_channel = None;
        }
        self.message_store.remove_all();
        self.rebuild_server_rail();
        self.rebuild_channel_list();
        self.channel_title.set_label("Select a channel");
    }

    // ---- Internal UI plumbing ----

    fn open_channel(self: &Rc<Self>, channel_id: Snowflake) {
        let title = {
            let state = self.state.borrow();
            find_channel(&state, channel_id)
                .map(|c| channel_label(&state, c))
                .unwrap_or_else(|| "channel".to_string())
        };
        self.state.borrow_mut().current_channel = Some(channel_id);
        self.channel_title.set_label(&title);
        self.message_store.remove_all();
        let _ = self.cmd.send(Command::OpenChannel { channel_id });
    }

    fn select_rail(self: &Rc<Self>, selection: RailSelection) {
        self.state.borrow_mut().selection = selection;
        if let RailSelection::Guild(guild_id) = selection {
            // GUILD_CREATE usually carries channels; fall back to REST.
            let needs_fetch = {
                let state = self.state.borrow();
                state
                    .guilds
                    .iter()
                    .find(|g| g.id == guild_id)
                    .map(|g| g.channels.is_empty())
                    .unwrap_or(false)
            };
            if needs_fetch {
                let _ = self.cmd.send(Command::FetchGuildChannels { guild_id });
            }
        }
        self.rebuild_channel_list();
        self.rebuild_server_rail_selection();
    }

    fn rebuild_server_rail(self: &Rc<Self>) {
        while let Some(child) = self.server_list.first_child() {
            self.server_list.remove(&child);
        }

        // DMs entry at the top.
        let dm_button = gtk::Button::with_label("DMs");
        dm_button.add_css_class("vesper-server-icon");
        dm_button.set_tooltip_text(Some("Direct Messages"));
        {
            let shell_weak = Rc::downgrade(self);
            dm_button.connect_clicked(move |_| {
                if let Some(shell) = shell_weak.upgrade() {
                    shell.select_rail(RailSelection::Dms);
                }
            });
        }
        self.server_list.append(&dm_button);

        let guilds: Vec<(Snowflake, String, Option<String>)> = {
            let state = self.state.borrow();
            state
                .guilds
                .iter()
                .map(|g| (g.id, g.display_name(), g.icon_url(96)))
                .collect()
        };
        for (guild_id, name, icon_url) in guilds {
            let button = gtk::Button::new();
            button.add_css_class("vesper-server-icon");
            button.set_tooltip_text(Some(&name));
            if let Some(url) = icon_url {
                let picture = gtk::Picture::new();
                picture.add_css_class("vesper-avatar");
                picture.set_size_request(40, 40);
                picture.set_content_fit(gtk::ContentFit::Cover);
                picture.set_overflow(gtk::Overflow::Hidden);
                avatar_cache::load_into(&picture, url);
                button.set_child(Some(&picture));
            } else {
                let initials: String = name
                    .split_whitespace()
                    .filter_map(|w| w.chars().next())
                    .take(2)
                    .collect();
                button.set_label(if initials.is_empty() { "?" } else { &initials });
            }
            {
                let shell_weak = Rc::downgrade(self);
                button.connect_clicked(move |_| {
                    if let Some(shell) = shell_weak.upgrade() {
                        shell.select_rail(RailSelection::Guild(guild_id));
                    }
                });
            }
            self.server_list.append(&button);
        }
        self.rebuild_server_rail_selection();
    }

    fn rebuild_server_rail_selection(self: &Rc<Self>) {
        let selection = self.state.borrow().selection;
        let mut child = self.server_list.first_child();
        let mut index: usize = 0;
        while let Some(widget) = child {
            let selected = match selection {
                RailSelection::Dms => index == 0,
                RailSelection::Guild(id) => {
                    index > 0
                        && self
                            .state
                            .borrow()
                            .guilds
                            .get(index - 1)
                            .map(|g| g.id == id)
                            .unwrap_or(false)
                }
            };
            if selected {
                widget.add_css_class("selected");
            } else {
                widget.remove_css_class("selected");
            }
            child = widget.next_sibling();
            index += 1;
        }
    }

    fn rebuild_channel_list(self: &Rc<Self>) {
        while let Some(row) = self.channel_list.row_at_index(0) {
            self.channel_list.remove(&row);
        }

        let state = self.state.borrow();
        match state.selection {
            RailSelection::Dms => {
                self.sidebar_title.set_label("Direct Messages");
                for channel in &state.private_channels {
                    self.append_channel_row(&channel_label(&state, channel), channel.id, false);
                }
            }
            RailSelection::Guild(guild_id) => {
                let Some(guild) = state.guilds.iter().find(|g| g.id == guild_id) else {
                    return;
                };
                self.sidebar_title.set_label(&guild.display_name());

                let mut channels: Vec<&Channel> = guild.channels.iter().collect();
                channels.sort_by_key(|c| (c.parent_id, c.position.unwrap_or(0), c.id));

                // Categories first (as labels), then their text channels;
                // then uncategorized text channels.
                let categories: Vec<&Channel> = channels
                    .iter()
                    .copied()
                    .filter(|c| c.kind == ChannelType::GuildCategory)
                    .collect();
                let texts = |parent: Option<Snowflake>| -> Vec<&Channel> {
                    channels
                        .iter()
                        .copied()
                        .filter(|c| {
                            matches!(
                                c.kind,
                                ChannelType::GuildText | ChannelType::GuildAnnouncement
                            ) && c.parent_id == parent
                        })
                        .collect()
                };

                for channel in texts(None) {
                    self.append_channel_row(
                        &format!("#  {}", channel.display_name()),
                        channel.id,
                        false,
                    );
                }
                for category in categories {
                    let rows = texts(Some(category.id));
                    if rows.is_empty() {
                        continue;
                    }
                    self.append_category_label(&category.display_name().to_uppercase());
                    for channel in rows {
                        self.append_channel_row(
                            &format!("#  {}", channel.display_name()),
                            channel.id,
                            false,
                        );
                    }
                }
            }
        }
    }

    fn append_category_label(&self, text: &str) {
        let label = gtk::Label::new(Some(text));
        label.add_css_class("vesper-category-label");
        label.set_xalign(0.0);
        let row = gtk::ListBoxRow::new();
        row.set_child(Some(&label));
        row.set_activatable(false);
        row.set_selectable(false);
        self.channel_list.append(&row);
    }

    fn append_channel_row(&self, text: &str, id: Snowflake, _selected: bool) {
        let label = gtk::Label::new(Some(text));
        label.set_xalign(0.0);
        label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        let row = gtk::ListBoxRow::new();
        row.add_css_class("vesper-channel-row");
        row.set_child(Some(&label));
        unsafe {
            row.set_data("channel-id", id.0);
        }
        self.channel_list.append(&row);
    }

    fn is_at_bottom(&self) -> bool {
        let adj = self.message_scroll.vadjustment();
        adj.value() + adj.page_size() >= adj.upper() - 48.0
    }

    fn scroll_to_bottom(&self) {
        // VERIFY: bottom-stick behavior while live messages arrive needs a
        // running session on a real display.
        let adj = self.message_scroll.vadjustment();
        // Defer until the ListView has resized for the new items.
        glib::idle_add_local_once(glib::clone!(
            #[weak]
            adj,
            move || {
                adj.set_value(adj.upper() - adj.page_size());
            }
        ));
    }
}

fn find_channel(state: &State, id: Snowflake) -> Option<&Channel> {
    state
        .private_channels
        .iter()
        .chain(state.guilds.iter().flat_map(|g| g.channels.iter()))
        .find(|c| c.id == id)
}

/// Channel label, resolving DM recipient names through the READY user list.
fn channel_label(state: &State, channel: &Channel) -> String {
    match channel.kind {
        ChannelType::Dm | ChannelType::GroupDm => {
            if let Some(name) = &channel.name {
                if !name.is_empty() {
                    return name.clone();
                }
            }
            if let Some(recipients) = &channel.recipients {
                let names: Vec<String> = recipients.iter().map(|u| u.display_name()).collect();
                if !names.is_empty() {
                    return names.join(", ");
                }
            }
            if let Some(ids) = &channel.recipient_ids {
                let names: Vec<String> = ids
                    .iter()
                    .filter_map(|id| state.users.get(id).map(|u| u.display_name()))
                    .collect();
                if !names.is_empty() {
                    return names.join(", ");
                }
            }
            debug!("DM channel {} has no resolvable name", channel.id);
            "Direct message".to_string()
        }
        _ => format!("# {}", channel.display_name()),
    }
}
