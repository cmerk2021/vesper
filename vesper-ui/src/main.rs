//! Vesper: a native, beautiful Discord client for Linux.

mod avatar_cache;
mod login;
mod message_list;
mod platform;
mod shell;
mod theme_manager;

use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gio, glib};
use tracing::info;
use vesper_core::{Command, Event};

const APP_ID: &str = "io.github.cmerk2021.Vesper";

fn main() -> glib::ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "vesper=info,vesper_core=info,vesper_ui=info".into()),
        )
        .init();

    let app = gtk::Application::builder().application_id(APP_ID).build();
    app.connect_activate(activate);
    app.run()
}

fn activate(app: &gtk::Application) {
    if let Some(window) = app.active_window() {
        window.present();
        return;
    }
    build_ui(app);
}

fn build_ui(app: &gtk::Application) {
    let core = vesper_core::session::spawn();
    let cmd = core.commands.clone();

    // Theme: load the configured theme and watch for live edits.
    let config = vesper_core::config::AppConfig::load();
    let themes = theme_manager::ThemeManager::new(&config.theme);
    themes.apply();
    themes.watch();

    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title("Vesper")
        .default_width(1240)
        .default_height(780)
        .build();
    window.add_css_class("vesper-window");

    // Client-side headerbar (plain GTK4, no libadwaita).
    let header = gtk::HeaderBar::new();
    header.add_css_class("vesper-headerbar");
    let title = gtk::Label::new(Some("Vesper"));
    title.add_css_class("vesper-author");
    header.set_title_widget(Some(&title));

    let menu = gio::Menu::new();
    menu.append(Some("Log out"), Some("app.logout"));
    let menu_button = gtk::MenuButton::new();
    menu_button.set_icon_name("open-menu-symbolic");
    menu_button.set_menu_model(Some(&menu));
    header.pack_end(&menu_button);
    window.set_titlebar(Some(&header));

    // Root stack: login <-> shell.
    let stack = gtk::Stack::new();
    stack.add_css_class("vesper-root");
    stack.set_transition_type(gtk::StackTransitionType::Crossfade);

    let login_view = login::LoginView::new(cmd.clone());
    let shell_view = shell::Shell::new(cmd.clone());
    let loading = gtk::Spinner::new();
    loading.set_spinning(true);
    loading.set_halign(gtk::Align::Center);
    loading.set_valign(gtk::Align::Center);
    loading.set_size_request(32, 32);

    stack.add_named(&loading, Some("loading"));
    stack.add_named(&login_view.root, Some("login"));
    stack.add_named(&shell_view.root, Some("shell"));
    stack.set_visible_child_name("loading");
    window.set_child(Some(&stack));

    // Platform integration stubs (blur etc. land in a later pass).
    let platform = platform::detect();
    if let Some(theme) = themes.current_theme() {
        platform.apply_blur(window.upcast_ref(), theme.blur, theme.blur_strength);
    }

    // "Log out" action.
    let logout = gio::SimpleAction::new("logout", None);
    {
        let cmd = cmd.clone();
        logout.connect_activate(move |_, _| {
            let _ = cmd.send(Command::Logout);
        });
    }
    app.add_action(&logout);

    // Consume core events on the GTK main loop.
    let events = core.events.clone();
    let stack_weak = stack.downgrade();
    let login_rc = login_view.clone();
    let shell_rc = shell_view.clone();
    glib::spawn_future_local(async move {
        while let Ok(event) = events.recv().await {
            let Some(stack) = stack_weak.upgrade() else {
                break;
            };
            handle_event(event, &stack, &login_rc, &shell_rc);
        }
    });

    let _ = cmd.send(Command::Init);
    window.present();
}

fn handle_event(
    event: Event,
    stack: &gtk::Stack,
    login_view: &Rc<login::LoginView>,
    shell_view: &Rc<shell::Shell>,
) {
    match event {
        Event::NeedsLogin { error } => {
            login_view.reset();
            login_view.set_error(error.as_deref());
            stack.set_visible_child_name("login");
        }
        Event::LoggingIn => {
            stack.set_visible_child_name("loading");
        }
        Event::Ready {
            user,
            guilds,
            private_channels,
            users,
        } => {
            info!("ready as {}", user.display_name());
            shell_view.on_ready(guilds, private_channels, users);
            stack.set_visible_child_name("shell");
        }
        Event::GatewayConnected => {}
        Event::GatewayDisconnected { reconnecting } => {
            info!(reconnecting, "gateway disconnected");
        }
        Event::GuildCreate(guild) => shell_view.on_guild_create(*guild),
        Event::ChannelCreate(c) | Event::ChannelUpdate(c) => shell_view.on_channel_event(*c, false),
        Event::ChannelDelete(c) => shell_view.on_channel_event(*c, true),
        Event::MessageCreate(message) => shell_view.on_message_create(&message),
        Event::MessageUpdate(_) => {
            // Edited-message rendering is pass 2; the plain-text list keeps
            // the original content for now.
        }
        Event::MessageDelete { channel_id, id } => shell_view.on_message_delete(channel_id, id),
        Event::TypingStart { .. } => {
            // Typing indicators are explicitly out of scope for this pass.
        }
        Event::MessagesLoaded {
            channel_id,
            messages,
        } => shell_view.on_messages_loaded(channel_id, messages),
        Event::GuildChannelsLoaded { guild_id, channels } => {
            shell_view.on_guild_channels_loaded(guild_id, channels)
        }
        Event::LoggedOut => {
            shell_view.clear();
            login::clear_webview_data();
        }
        Event::Error { message } => {
            tracing::warn!("core error: {message}");
        }
    }
}
