//! Asynchronous CDN image fetching with an in-memory cache. Never blocks the
//! GTK main loop; widgets show a themed placeholder until the image arrives.

use std::cell::RefCell;
use std::collections::HashMap;

use gtk::gdk;
use gtk::glib;
use gtk::prelude::*;
use soup::prelude::*;
use tracing::debug;

thread_local! {
    static CACHE: RefCell<HashMap<String, gdk::Texture>> = RefCell::new(HashMap::new());
    static SESSION: soup::Session = soup::Session::new();
}

/// Loads `url` into `picture` asynchronously. While loading (or on failure)
/// the picture keeps whatever placeholder styling its CSS class provides.
pub fn load_into(picture: &gtk::Picture, url: String) {
    // Tag the widget with the URL so a recycled list item that has since been
    // re-bound to another message does not get a stale avatar.
    picture.set_widget_name(&url);

    let cached = CACHE.with(|c| c.borrow().get(&url).cloned());
    if let Some(texture) = cached {
        picture.set_paintable(Some(&texture));
        return;
    }
    picture.set_paintable(gdk::Paintable::NONE);

    let weak = picture.downgrade();
    glib::spawn_future_local(async move {
        let Some(texture) = fetch_texture(&url).await else {
            return;
        };
        CACHE.with(|c| c.borrow_mut().insert(url.clone(), texture.clone()));
        if let Some(picture) = weak.upgrade() {
            if picture.widget_name() == url {
                picture.set_paintable(Some(&texture));
            }
        }
    });
}

async fn fetch_texture(url: &str) -> Option<gdk::Texture> {
    let message = soup::Message::new("GET", url).ok()?;
    let session = SESSION.with(|s| s.clone());
    let bytes = match session
        .send_and_read_future(&message, glib::Priority::LOW)
        .await
    {
        Ok(bytes) => bytes,
        Err(e) => {
            debug!("image fetch failed: {e}");
            return None;
        }
    };
    if message.status() != soup::Status::Ok {
        debug!(status = ?message.status(), "image fetch returned non-OK status");
        return None;
    }
    match gdk::Texture::from_bytes(&bytes) {
        Ok(texture) => Some(texture),
        Err(e) => {
            debug!("image decode failed: {e}");
            None
        }
    }
}
