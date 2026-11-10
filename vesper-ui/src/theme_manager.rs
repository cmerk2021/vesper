//! Loads themes, applies the generated CSS to the display, and hot-reloads
//! on file changes via `notify` (debounced). Invalid themes log a warning
//! and keep the previous theme.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gtk::{gdk, glib};
use notify::Watcher;
use tracing::{info, warn};
use vesper_core::theme;

pub struct ThemeManager {
    theme_name: String,
    provider: RefCell<Option<gtk::CssProvider>>,
    current: RefCell<Option<theme::ResolvedTheme>>,
    // Kept alive for the lifetime of the app so hot reload keeps working.
    _watcher: RefCell<Option<notify::RecommendedWatcher>>,
}

impl ThemeManager {
    pub fn new(theme_name: &str) -> Rc<Self> {
        Rc::new(ThemeManager {
            theme_name: theme_name.to_string(),
            provider: RefCell::new(None),
            current: RefCell::new(None),
            _watcher: RefCell::new(None),
        })
    }

    /// The currently applied theme tokens (after `apply`).
    pub fn current_theme(&self) -> Option<theme::Theme> {
        self.current.borrow().as_ref().map(|r| r.theme.clone())
    }

    /// Resolves and applies the configured theme. On failure the previous
    /// theme (or GTK defaults) stays active.
    pub fn apply(&self) {
        let themes_dir = vesper_core::config::themes_dir();
        let resolved = match theme::resolve_theme(&themes_dir, &self.theme_name) {
            Ok(r) => r,
            Err(e) => {
                warn!(
                    "failed to load theme '{}': {e}; keeping previous theme",
                    self.theme_name
                );
                return;
            }
        };
        let css = resolved.css();

        let Some(display) = gdk::Display::default() else {
            warn!("no display; cannot apply theme");
            return;
        };

        // Load into a fresh provider and only swap it in if the CSS parses
        // cleanly, so a broken override.css never clobbers the active theme.
        let provider = gtk::CssProvider::new();
        let error_count = Rc::new(std::cell::Cell::new(0u32));
        {
            let error_count = error_count.clone();
            provider.connect_parsing_error(move |_, _, error| {
                warn!("theme CSS parse error: {error}");
                error_count.set(error_count.get() + 1);
            });
        }
        provider.load_from_string(&css);
        if error_count.get() > 0 {
            warn!(
                "theme '{}' produced invalid CSS; keeping previous theme",
                resolved.name
            );
            return;
        }

        if let Some(old) = self.provider.borrow_mut().take() {
            gtk::style_context_remove_provider_for_display(&display, &old);
        }
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_USER,
        );
        self.provider.borrow_mut().replace(provider);
        info!("applied theme '{}'", resolved.name);
        self.current.borrow_mut().replace(resolved);
    }

    /// Watches the themes directory and re-applies the theme on changes,
    /// debounced to avoid thrashing while files are being saved.
    pub fn watch(self: &Rc<Self>) {
        let themes_dir = vesper_core::config::themes_dir();
        if let Err(e) = std::fs::create_dir_all(&themes_dir) {
            warn!("cannot create themes dir: {e}");
            return;
        }
        if let Err(e) = theme::write_example_themes(&themes_dir) {
            warn!("cannot write example themes: {e}");
        }

        let (tx, rx) = async_channel::unbounded::<()>();
        let mut watcher =
            match notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
                if res.is_ok() {
                    let _ = tx.send_blocking(());
                }
            }) {
                Ok(w) => w,
                Err(e) => {
                    warn!("theme hot reload unavailable: {e}");
                    return;
                }
            };
        if let Err(e) = watcher.watch(&themes_dir, notify::RecursiveMode::Recursive) {
            warn!("cannot watch themes dir: {e}");
            return;
        }
        self._watcher.borrow_mut().replace(watcher);

        let manager = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            while rx.recv().await.is_ok() {
                // Debounce: swallow further events for a short window.
                glib::timeout_future(Duration::from_millis(300)).await;
                while rx.try_recv().is_ok() {}
                if let Some(manager) = manager.upgrade() {
                    info!("theme files changed; reloading");
                    manager.apply();
                } else {
                    break;
                }
            }
        });
    }
}
