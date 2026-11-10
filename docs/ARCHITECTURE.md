# Vesper Architecture

Vesper is a native GTK4 Discord client for Linux, split into two crates in a
Cargo workspace:

```
vesper-core/   # Discord protocol + config/theme logic. No GTK dependency.
vesper-ui/     # GTK4 application (binary `vesper`). Depends on vesper-core.
```

## Threading & channel design

`vesper-core` runs on a **tokio runtime on a background thread**. The UI talks
to it exclusively through two channels:

- **`Command`** (UI → core): an unbounded tokio `mpsc` channel. Examples:
  `Init`, `Login { token }`, `Logout`, `OpenChannel { channel_id }`,
  `SendMessage { channel_id, content, reply_to }`, `FetchGuildChannels`.
- **`Event`** (core → UI): an unbounded `async_channel`. The UI consumes it on
  the GTK main loop via `glib::spawn_future_local`. Examples: `NeedsLogin`,
  `Ready { .. }`, `GuildCreate`, `MessageCreate`, `MessagesLoaded`,
  `GatewayConnected` / `GatewayDisconnected`, `LoggedOut`, `Error`.

The core never touches GTK; the UI never touches sockets or the keyring.

## vesper-core modules

- `models` — tolerant serde models (`Snowflake`, `User`, `Guild`, `Channel`,
  `Message`, gateway payloads). Every field is `Option<T>` or
  `#[serde(default)]`; unknown enum values map to catch-all variants;
  snowflakes deserialize from string or number.
- `gateway` — Discord gateway v10 (JSON, no compression): HELLO, heartbeat
  with jitter and ACK tracking, user-account IDENTIFY (desktop properties, no
  intents), RESUME, INVALID_SESSION handling, close-code policy, exponential
  backoff with jitter. `gateway::backoff` is pure logic and unit tested.
- `rest` — reqwest client: current user, channel messages (`limit`/`before`),
  send message (with `message_reference`), guild channels, 429 `retry_after`
  handling, `Authorization`/`User-Agent`/`X-Super-Properties` headers.
- `props` — shared desktop "super properties" identity object.
- `session` — owns the runtime thread; wires commands → gateway/REST → events;
  stores/loads/deletes the token via `oo7` (Secret Service).
- `theme` — theme token parsing (TOML, all defaulted), GTK CSS generation,
  built-in themes (embedded via `include_str!`), user-theme resolution.
- `config` — XDG paths via `directories` plus `config.toml`.

## vesper-ui modules

- `main.rs` — `gtk::Application` (`io.github.cmerk2021.Vesper`), headerbar,
  login/shell stack, event-loop bridge, `app.logout` action.
- `login.rs` — webview login (`webkit6`: user script intercepts fetch/XHR
  `Authorization` headers on `/api/` requests and posts them to Rust) plus a
  paste-token fallback; persistent `NetworkSession` data dir; data cleared on
  logout.
- `shell.rs` — server rail / channel list (categories + DMs) / message area /
  input (Enter sends, Shift+Enter newline).
- `message_list.rs` — `gtk::ListView` + `gio::ListStore` of `MessageObject`
  GObjects; 7-minute same-author grouping; bottom-stick auto-scroll.
- `avatar_cache.rs` — async CDN fetch (libsoup 3) with in-memory cache.
- `theme_manager.rs` — applies generated CSS via `gtk::CssProvider` at
  `STYLE_PROVIDER_PRIORITY_USER`; hot reload with `notify` (debounced);
  invalid TOML/CSS keeps the previous theme.
- `platform/` — `PlatformIntegration` trait + `NoopPlatform` stub (blur,
  global menu, voice, screen share are later passes).

## Theme pipeline

`theme.toml` tokens → `Theme` (serde, all defaulted) → generated GTK CSS
(`@define-color` + rules keyed off `.vesper-*` classes; spacing derived from
`spacing_unit` × density) → `override.css` appended → `CssProvider`. A theme
with `window_opacity < 1.0` makes the window background transparent so the
compositor can blend (and later blur) behind it.
