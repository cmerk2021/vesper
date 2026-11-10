# Vesper

**Vesper** is a native, beautiful Discord client for Linux, written in Rust
with GTK 4. It connects to Discord with your regular user account and aims for
a familiar layout — server rail, channel list, messages — with deep theming,
generous spacing, and desktop integration.

> [!WARNING]
> **Discord ToS risk.** Vesper is an unofficial client that logs in with a
> user account ("self-client"). Automating or using a user account outside the
> official clients can violate Discord's Terms of Service and may get your
> account flagged or banned. **Use Vesper at your own risk.**

## Features (pass 1)

- Native GTK4 UI (no Electron, no libadwaita) with a client-side headerbar
- Login via an embedded WebKitGTK webview (token captured from the login
  flow's own API requests) or by pasting a token; tokens stored in the system
  keyring via the Secret Service (`oo7`)
- Discord gateway v10 with resume, heartbeat ACK tracking and exponential
  backoff; tolerant parsing that never breaks on new/unknown API fields
- Message view with author grouping, async avatar loading and auto-scroll;
  Enter sends, Shift+Enter inserts a newline
- TOML theme engine with live hot reload and three built-in themes:
  **Vesper Dark** (default), **Vesper Light**, **Vesper Glass** (translucent)

## Building

Dependencies (Ubuntu 24.04 names):

```sh
sudo apt install build-essential pkg-config libssl-dev \
  libgtk-4-dev libglib2.0-dev libwebkitgtk-6.0-dev \
  libjavascriptcoregtk-6.0-dev libsoup-3.0-dev libgraphene-1.0-dev
```

Then:

```sh
cargo build --release
./target/release/vesper
```

Run the checks the CI runs:

```sh
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## Writing a theme

Themes live in `~/.config/vesper/themes/<name>/` and consist of a
`theme.toml` plus an optional `override.css` (raw GTK CSS appended after the
generated CSS). Select the theme in `~/.config/vesper/config.toml`:

```toml
theme = "my-theme"
```

A user theme directory with the same name as a built-in theme
(`vesper-dark`, `vesper-light`, `vesper-glass`) takes precedence over the
built-in. On first run Vesper copies the built-in themes to
`~/.config/vesper/themes/examples/` as starting points. Editing a theme file
re-applies it live; invalid TOML or CSS logs a warning and keeps the previous
theme.

Every token is optional and falls back to the Vesper Dark defaults:

```toml
# Window
window_opacity = 1.0    # 0.0..=1.0; < 1.0 makes the window background transparent
blur = false            # ask the compositor for blur (platform layer, later pass)
blur_strength = 20.0    # blur hint for the platform layer

# Shape & spacing
radius_sm = 4.0         # small corner radius (px) — channel rows
radius_md = 8.0         # medium radius — inputs, buttons
radius_lg = 16.0        # large radius — server icons
spacing_unit = 8.0      # base spacing unit (px); all padding derives from it
density = "cozy"        # "cozy" | "compact" | "comfortable"

# Typography
font_ui = "Sans"        # UI font family
font_message = "Sans"   # message text font family
font_mono = "Monospace" # monospace font family
font_size_base = 14.0   # base font size (px)

# Misc
avatar_size = 40        # avatar size (px)

[colors]                # any CSS color: #rrggbb, rgba(...), named colors
bg_base = "#121318"     # window / message area background
bg_surface = "#1a1c23"  # sidebar background
bg_elevated = "#23262f" # inputs, hovered rows, placeholders
fg_primary = "#e8eaf0"  # main text
fg_muted = "#8a8f9e"    # secondary text (timestamps, categories)
accent = "#7d8cff"      # accent (buttons, selection)
accent_fg = "#ffffff"   # text on accent
danger = "#f0556c"      # errors / destructive
border = "#2c2f3a"      # separators and outlines
mention_bg = "rgba(125, 140, 255, 0.14)"  # mention / selection wash
```

All Vesper widgets carry stable CSS classes (`.vesper-message`,
`.vesper-server-icon`, `.vesper-channel-row`, `.vesper-avatar`,
`.vesper-input`, …) so `override.css` can restyle anything the tokens don't
cover.

## Tagging a release

Releases are built by CI from tags:

```sh
git tag v0.1.0
git push origin v0.1.0
```

The `Release` workflow generates `cargo-sources.json` from `Cargo.lock`,
builds the Flatpak (`io.github.cmerk2021.Vesper`) against the GNOME 47
runtime, and attaches `vesper.flatpak` to a GitHub Release for the tag. Tags
containing a `-` (e.g. `v0.2.0-rc1`) are marked as prereleases.

## License

GPL-3.0-or-later. See [LICENSE](LICENSE).
