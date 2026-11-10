//! Login screen: webview-based login (token sniffed from the first
//! authorized `/api/` request) plus a paste-token fallback.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;
use tracing::{info, warn};
use vesper_core::Command;
use webkit6::prelude::*;

const TOKEN_HANDLER: &str = "vesperToken";

/// Injected at document start. Wraps `window.fetch` and
/// `XMLHttpRequest.prototype.setRequestHeader`; the first `/api/` request
/// carrying an `Authorization` header posts it to Rust. localStorage is
/// intentionally NOT read (Discord removes the token from it).
const TOKEN_SNIFFER_JS: &str = r#"
(function () {
    'use strict';
    let sent = false;
    function post(token) {
        if (sent || !token || typeof token !== 'string') { return; }
        if (token.toLowerCase().startsWith('bearer ')) { return; }
        sent = true;
        try { window.webkit.messageHandlers.vesperToken.postMessage(token); } catch (e) {}
    }
    function authFromHeaders(headers) {
        if (!headers) { return null; }
        try {
            if (typeof Headers !== 'undefined' && headers instanceof Headers) {
                return headers.get('Authorization') || headers.get('authorization');
            }
            if (Array.isArray(headers)) {
                for (const pair of headers) {
                    if (pair && String(pair[0]).toLowerCase() === 'authorization') { return pair[1]; }
                }
                return null;
            }
            for (const key in headers) {
                if (key.toLowerCase() === 'authorization') { return headers[key]; }
            }
        } catch (e) {}
        return null;
    }
    const origFetch = window.fetch;
    window.fetch = function (input, init) {
        try {
            let url = '';
            let auth = null;
            if (typeof input === 'string') { url = input; }
            else if (input && typeof input.url === 'string') {
                url = input.url;
                if (input.headers) { auth = authFromHeaders(input.headers); }
            }
            if (init && init.headers) { auth = authFromHeaders(init.headers) || auth; }
            if (auth && url.indexOf('/api/') !== -1) { post(auth); }
        } catch (e) {}
        return origFetch.apply(this, arguments);
    };
    const origOpen = XMLHttpRequest.prototype.open;
    XMLHttpRequest.prototype.open = function (method, url) {
        try { this.__vesperUrl = String(url); } catch (e) {}
        return origOpen.apply(this, arguments);
    };
    const origSetHeader = XMLHttpRequest.prototype.setRequestHeader;
    XMLHttpRequest.prototype.setRequestHeader = function (name, value) {
        try {
            if (String(name).toLowerCase() === 'authorization' &&
                this.__vesperUrl && this.__vesperUrl.indexOf('/api/') !== -1) {
                post(value);
            }
        } catch (e) {}
        return origSetHeader.apply(this, arguments);
    };
})();
"#;

/// Loose sanity check that a sniffed string plausibly is a user token.
fn looks_like_token(raw: &str) -> bool {
    let t = raw.trim().trim_matches('"');
    if t.len() < 40 || t.len() > 300 {
        return false;
    }
    if t.chars().any(|c| c.is_whitespace()) {
        return false;
    }
    if t.to_ascii_lowercase().starts_with("bearer") || t.starts_with("Bot ") {
        return false;
    }
    t.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

pub struct LoginView {
    pub root: gtk::Stack,
    error_label: gtk::Label,
    token_entry: gtk::PasswordEntry,
    webview_page: gtk::Box,
    webview: RefCell<Option<webkit6::WebView>>,
    cmd: vesper_core::CommandSender,
}

impl LoginView {
    pub fn new(cmd: vesper_core::CommandSender) -> Rc<Self> {
        let root = gtk::Stack::new();
        root.set_transition_type(gtk::StackTransitionType::Crossfade);

        // ---- Choice / paste page ----
        let page = gtk::Box::new(gtk::Orientation::Vertical, 0);
        page.add_css_class("vesper-login");
        page.set_valign(gtk::Align::Center);
        page.set_halign(gtk::Align::Center);

        let title = gtk::Label::new(Some("Vesper"));
        title.add_css_class("vesper-login-title");
        page.append(&title);

        let subtitle = gtk::Label::new(Some(
            "A native Discord client for Linux.\nUser-account clients can violate Discord's Terms of Service — use at your own risk.",
        ));
        subtitle.add_css_class("vesper-muted");
        subtitle.set_justify(gtk::Justification::Center);
        page.append(&subtitle);

        let browser_button = gtk::Button::with_label("Log in with Discord (browser window)");
        browser_button.add_css_class("vesper-accent-button");
        page.append(&browser_button);

        let separator = gtk::Label::new(Some("— or paste a token —"));
        separator.add_css_class("vesper-muted");
        page.append(&separator);

        let token_entry = gtk::PasswordEntry::new();
        token_entry.set_show_peek_icon(true);
        token_entry.add_css_class("vesper-input");
        token_entry.set_property("placeholder-text", "Discord token");
        page.append(&token_entry);

        let login_button = gtk::Button::with_label("Log in");
        login_button.add_css_class("vesper-accent-button");
        page.append(&login_button);

        let error_label = gtk::Label::new(None);
        error_label.add_css_class("vesper-login-error");
        error_label.set_wrap(true);
        error_label.set_visible(false);
        page.append(&error_label);

        // ---- Webview page (populated lazily) ----
        let webview_page = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let back_button = gtk::Button::with_label("Back");
        webview_page.append(&back_button);

        root.add_named(&page, Some("choose"));
        root.add_named(&webview_page, Some("web"));
        root.set_visible_child_name("choose");

        let view = Rc::new(LoginView {
            root,
            error_label,
            token_entry,
            webview_page,
            webview: RefCell::new(None),
            cmd,
        });

        {
            let view = view.clone();
            login_button.connect_clicked(move |_| view.submit_pasted_token());
        }
        {
            let view = view.clone();
            view.token_entry
                .clone()
                .connect_activate(move |_| view.submit_pasted_token());
        }
        {
            let view = view.clone();
            browser_button.connect_clicked(move |_| view.open_webview());
        }
        {
            let view = view.clone();
            back_button.connect_clicked(move |_| view.close_webview());
        }

        view
    }

    pub fn set_error(&self, error: Option<&str>) {
        match error {
            Some(msg) => {
                self.error_label.set_label(msg);
                self.error_label.set_visible(true);
            }
            None => self.error_label.set_visible(false),
        }
    }

    /// Called when the app returns to the login screen.
    pub fn reset(&self) {
        self.close_webview();
        self.token_entry.set_text("");
    }

    fn submit_pasted_token(&self) {
        let token = self.token_entry.text().trim().to_string();
        if token.is_empty() {
            self.set_error(Some("Please paste a token."));
            return;
        }
        self.set_error(None);
        let _ = self.cmd.send(Command::Login { token });
    }

    fn open_webview(self: &Rc<Self>) {
        // VERIFY: webview login flow (script injection, token capture, page
        // teardown) needs a real display + Discord login; not testable headless.
        if self.webview.borrow().is_some() {
            self.root.set_visible_child_name("web");
            return;
        }

        let data_dir = vesper_core::config::webview_data_dir();
        let cache_dir = data_dir.join("cache");
        let _ = std::fs::create_dir_all(&data_dir);
        let session = webkit6::NetworkSession::new(
            Some(&data_dir.to_string_lossy()),
            Some(&cache_dir.to_string_lossy()),
        );

        let ucm = webkit6::UserContentManager::new();
        let script = webkit6::UserScript::new(
            TOKEN_SNIFFER_JS,
            webkit6::UserContentInjectedFrames::TopFrame,
            webkit6::UserScriptInjectionTime::Start,
            &[],
            &[],
        );
        ucm.add_script(&script);

        let got_token = Rc::new(Cell::new(false));
        if !ucm.register_script_message_handler(TOKEN_HANDLER, None) {
            warn!("could not register the token message handler");
        }
        {
            let view = Rc::downgrade(self);
            let got_token = got_token.clone();
            ucm.connect_script_message_received(Some(TOKEN_HANDLER), move |_, value| {
                if got_token.get() {
                    return;
                }
                let raw = value.to_str().to_string();
                if !looks_like_token(&raw) {
                    // Not a token (never log the value itself).
                    return;
                }
                got_token.set(true);
                info!("captured login token from webview");
                if let Some(view) = view.upgrade() {
                    let token = raw.trim().trim_matches('"').to_string();
                    // Tear the webview down outside the signal handler.
                    let view2 = view.clone();
                    glib::idle_add_local_once(move || {
                        view2.close_webview();
                        let _ = view2.cmd.send(Command::Login { token });
                    });
                }
            });
        }

        let webview = webkit6::WebView::builder()
            .network_session(&session)
            .user_content_manager(&ucm)
            .build();
        webview.set_vexpand(true);
        webview.set_hexpand(true);
        webview.load_uri("https://discord.com/login");

        self.webview_page.append(&webview);
        self.webview.borrow_mut().replace(webview);
        self.root.set_visible_child_name("web");
    }

    fn close_webview(&self) {
        if let Some(webview) = self.webview.borrow_mut().take() {
            self.webview_page.remove(&webview);
        }
        self.root.set_visible_child_name("choose");
    }
}

/// Clears persistent webview data (cookies, storage). Called on logout.
/// The webview is never alive when this runs.
pub fn clear_webview_data() {
    let dir = vesper_core::config::webview_data_dir();
    if dir.exists() {
        if let Err(e) = std::fs::remove_dir_all(&dir) {
            warn!("failed to clear webview data: {e}");
        }
    }
}
