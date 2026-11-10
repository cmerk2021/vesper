//! Theme tokens (`theme.toml`), defaults, and GTK CSS generation.
//!
//! Themes live in `~/.config/vesper/themes/<name>/` with a `theme.toml` and
//! an optional `override.css`. Built-in themes are embedded in the binary;
//! user themes with the same name take precedence.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::CoreError;

/// Built-in themes as `(name, theme.toml contents)`.
pub const BUILTIN_THEMES: &[(&str, &str)] = &[
    ("vesper-dark", include_str!("../themes/vesper-dark.toml")),
    ("vesper-light", include_str!("../themes/vesper-light.toml")),
    ("vesper-glass", include_str!("../themes/vesper-glass.toml")),
];

pub const DEFAULT_THEME: &str = "vesper-dark";

/// Layout density. Unknown values fall back to `cozy`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Density {
    #[default]
    Cozy,
    Compact,
    Comfortable,
    #[serde(other)]
    Other,
}

impl Density {
    /// Spacing multiplier for the density.
    pub fn multiplier(self) -> f64 {
        match self {
            Density::Compact => 0.75,
            Density::Cozy | Density::Other => 1.0,
            Density::Comfortable => 1.3,
        }
    }
}

/// Color tokens. Values are CSS color strings (`#rrggbb`, `rgba(...)`, ...).
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct Colors {
    pub bg_base: String,
    pub bg_surface: String,
    pub bg_elevated: String,
    pub fg_primary: String,
    pub fg_muted: String,
    pub accent: String,
    pub accent_fg: String,
    pub danger: String,
    pub border: String,
    pub mention_bg: String,
}

impl Default for Colors {
    fn default() -> Self {
        // Vesper Dark palette.
        Colors {
            bg_base: "#121318".into(),
            bg_surface: "#1a1c23".into(),
            bg_elevated: "#23262f".into(),
            fg_primary: "#e8eaf0".into(),
            fg_muted: "#8a8f9e".into(),
            accent: "#7d8cff".into(),
            accent_fg: "#ffffff".into(),
            danger: "#f0556c".into(),
            border: "#2c2f3a".into(),
            mention_bg: "rgba(125, 140, 255, 0.14)".into(),
        }
    }
}

/// All theme tokens. Every field is optional in TOML and falls back to the
/// Vesper Dark defaults.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct Theme {
    pub colors: Colors,
    /// 0.0..=1.0; values below 1.0 make the window background transparent.
    pub window_opacity: f64,
    /// Request compositor blur (consumed by the platform layer later).
    pub blur: bool,
    /// Blur strength hint for the platform layer; not used by CSS.
    pub blur_strength: f64,
    pub radius_sm: f64,
    pub radius_md: f64,
    pub radius_lg: f64,
    /// Base spacing unit in px; all paddings/margins derive from it.
    pub spacing_unit: f64,
    pub density: Density,
    pub font_ui: String,
    pub font_message: String,
    pub font_mono: String,
    pub font_size_base: f64,
    pub avatar_size: u32,
}

impl Default for Theme {
    fn default() -> Self {
        Theme {
            colors: Colors::default(),
            window_opacity: 1.0,
            blur: false,
            blur_strength: 20.0,
            radius_sm: 4.0,
            radius_md: 8.0,
            radius_lg: 16.0,
            spacing_unit: 8.0,
            density: Density::Cozy,
            font_ui: "Sans".into(),
            font_message: "Sans".into(),
            font_mono: "Monospace".into(),
            font_size_base: 14.0,
            avatar_size: 40,
        }
    }
}

impl Theme {
    /// Parses `theme.toml` content. Unknown keys are ignored; missing keys
    /// use defaults.
    pub fn from_toml(text: &str) -> Result<Theme, CoreError> {
        let mut theme: Theme = toml::from_str(text).map_err(|e| CoreError::Theme(e.to_string()))?;
        theme.window_opacity = theme.window_opacity.clamp(0.0, 1.0);
        Ok(theme)
    }

    /// Effective spacing in px for `n` spacing units, density applied.
    pub fn spacing(&self, n: f64) -> f64 {
        (self.spacing_unit * self.density.multiplier() * n).max(0.0)
    }

    /// Generates the GTK CSS for this theme.
    pub fn to_css(&self) -> String {
        let c = &self.colors;
        let sp = |n: f64| format!("{:.0}px", self.spacing(n));
        let mut css = String::new();

        for (name, value) in [
            ("vesper_bg_base", &c.bg_base),
            ("vesper_bg_surface", &c.bg_surface),
            ("vesper_bg_elevated", &c.bg_elevated),
            ("vesper_fg_primary", &c.fg_primary),
            ("vesper_fg_muted", &c.fg_muted),
            ("vesper_accent", &c.accent),
            ("vesper_accent_fg", &c.accent_fg),
            ("vesper_danger", &c.danger),
            ("vesper_border", &c.border),
            ("vesper_mention_bg", &c.mention_bg),
        ] {
            css.push_str(&format!("@define-color {name} {value};\n"));
        }

        let window_bg = if self.window_opacity < 1.0 {
            // Transparent window background so the compositor can blend
            // (and, later, blur) behind semi-transparent surfaces.
            // VERIFY: actual translucency (Vesper Glass) requires a
            // compositor; check visually on a real desktop.
            "transparent".to_string()
        } else {
            "@vesper_bg_base".to_string()
        };

        css.push_str(&format!(
            r#"
window.vesper-window {{
  background: {window_bg};
  color: @vesper_fg_primary;
  font-family: {font_ui};
  font-size: {font_size:.0}px;
}}
window.vesper-window > .vesper-root {{
  background-color: @vesper_bg_base;
}}
.vesper-headerbar {{
  background-color: @vesper_bg_surface;
  color: @vesper_fg_primary;
  border-bottom: 1px solid @vesper_border;
  min-height: {sp4};
}}
.vesper-server-rail {{
  background-color: @vesper_bg_base;
  padding: {sp1};
}}
.vesper-server-icon {{
  min-width: {icon}px;
  min-height: {icon}px;
  margin: {sp_half} 0;
  padding: 0;
  border-radius: {radius_lg:.0}px;
  background-color: @vesper_bg_elevated;
  color: @vesper_fg_primary;
}}
.vesper-server-icon:hover, .vesper-server-icon.selected {{
  background-color: @vesper_accent;
  color: @vesper_accent_fg;
}}
.vesper-sidebar {{
  background-color: @vesper_bg_surface;
  border-right: 1px solid @vesper_border;
}}
.vesper-sidebar-title {{
  font-weight: bold;
  padding: {sp2};
  border-bottom: 1px solid @vesper_border;
}}
.vesper-channel-list {{
  background: transparent;
  padding: {sp1};
}}
.vesper-category-label {{
  color: @vesper_fg_muted;
  font-size: {small:.0}px;
  font-weight: bold;
  padding: {sp2} {sp1} {sp_half} {sp1};
}}
.vesper-channel-row {{
  color: @vesper_fg_muted;
  border-radius: {radius_sm:.0}px;
  padding: {sp_half} {sp1};
}}
.vesper-channel-row:hover {{
  background-color: @vesper_bg_elevated;
  color: @vesper_fg_primary;
}}
.vesper-channel-row:selected {{
  background-color: @vesper_mention_bg;
  color: @vesper_fg_primary;
}}
.vesper-main {{
  background-color: @vesper_bg_base;
}}
.vesper-message-list {{
  background: transparent;
  padding: {sp1} 0;
}}
.vesper-message {{
  padding: {sp_half} {sp2};
  font-family: {font_message};
}}
.vesper-message-grouped {{
  padding-top: 0;
}}
.vesper-author {{
  font-weight: bold;
  color: @vesper_fg_primary;
}}
.vesper-timestamp {{
  color: @vesper_fg_muted;
  font-size: {small:.0}px;
}}
.vesper-content {{
  color: @vesper_fg_primary;
}}
.vesper-avatar {{
  border-radius: 9999px;
  background-color: @vesper_bg_elevated;
  min-width: {avatar}px;
  min-height: {avatar}px;
}}
.vesper-input-area {{
  padding: {sp2};
  background-color: @vesper_bg_base;
}}
.vesper-input {{
  background-color: @vesper_bg_elevated;
  color: @vesper_fg_primary;
  border: 1px solid @vesper_border;
  border-radius: {radius_md:.0}px;
  padding: {sp1} {sp2};
  caret-color: @vesper_accent;
}}
.vesper-login {{
  background-color: @vesper_bg_base;
  color: @vesper_fg_primary;
  padding: {sp4};
}}
.vesper-login-title {{
  font-size: {big:.0}px;
  font-weight: bold;
  margin-bottom: {sp2};
}}
.vesper-login-error {{
  color: @vesper_danger;
  margin-top: {sp1};
}}
.vesper-accent-button {{
  background-color: @vesper_accent;
  color: @vesper_accent_fg;
  border-radius: {radius_md:.0}px;
  padding: {sp1} {sp2};
}}
.vesper-muted {{
  color: @vesper_fg_muted;
}}
.vesper-mono {{
  font-family: {font_mono};
}}
"#,
            font_ui = self.font_ui,
            font_message = self.font_message,
            font_mono = self.font_mono,
            font_size = self.font_size_base,
            small = (self.font_size_base - 2.0).max(8.0),
            big = self.font_size_base * 1.7,
            icon = self.avatar_size + 8,
            avatar = self.avatar_size,
            radius_sm = self.radius_sm,
            radius_md = self.radius_md,
            radius_lg = self.radius_lg,
            sp_half = sp(0.5),
            sp1 = sp(1.0),
            sp2 = sp(2.0),
            sp4 = sp(4.0),
        ));

        css
    }
}

/// A resolved theme: parsed tokens plus optional raw `override.css`.
#[derive(Debug, Clone)]
pub struct ResolvedTheme {
    pub name: String,
    pub theme: Theme,
    pub override_css: Option<String>,
}

impl ResolvedTheme {
    /// Full CSS: generated token CSS followed by `override.css` if present.
    pub fn css(&self) -> String {
        let mut css = self.theme.to_css();
        if let Some(extra) = &self.override_css {
            css.push('\n');
            css.push_str(extra);
        }
        css
    }
}

/// Loads a theme by name: user theme directory first, then built-ins.
pub fn resolve_theme(themes_dir: &Path, name: &str) -> Result<ResolvedTheme, CoreError> {
    let dir = themes_dir.join(name);
    let toml_path = dir.join("theme.toml");
    if toml_path.is_file() {
        let text = std::fs::read_to_string(&toml_path)?;
        let theme = Theme::from_toml(&text)?;
        let override_css = std::fs::read_to_string(dir.join("override.css")).ok();
        return Ok(ResolvedTheme {
            name: name.to_string(),
            theme,
            override_css,
        });
    }
    for (builtin_name, text) in BUILTIN_THEMES {
        if *builtin_name == name {
            return Ok(ResolvedTheme {
                name: name.to_string(),
                theme: Theme::from_toml(text)?,
                override_css: None,
            });
        }
    }
    Err(CoreError::Theme(format!("theme '{name}' not found")))
}

/// Copies the built-in themes into `<themes_dir>/examples/<name>/theme.toml`
/// so users have editable starting points. Never overwrites existing files.
pub fn write_example_themes(themes_dir: &Path) -> std::io::Result<()> {
    for (name, text) in BUILTIN_THEMES {
        let dir = themes_dir.join("examples").join(name);
        std::fs::create_dir_all(&dir)?;
        let path = dir.join("theme.toml");
        if !path.exists() {
            std::fs::write(path, text)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_toml_gives_defaults() {
        let t = Theme::from_toml("").unwrap();
        assert_eq!(t, Theme::default());
        assert_eq!(t.window_opacity, 1.0);
        assert_eq!(t.density, Density::Cozy);
        assert_eq!(t.avatar_size, 40);
    }

    #[test]
    fn partial_toml_overrides_some_tokens() {
        let t = Theme::from_toml(
            r##"
window_opacity = 0.85
spacing_unit = 10
density = "compact"

[colors]
accent = "#ff0000"
"##,
        )
        .unwrap();
        assert_eq!(t.window_opacity, 0.85);
        assert_eq!(t.colors.accent, "#ff0000");
        // Untouched tokens keep defaults.
        assert_eq!(t.colors.bg_base, Colors::default().bg_base);
        assert_eq!(t.density, Density::Compact);
        assert_eq!(t.spacing(1.0), 7.5);
    }

    #[test]
    fn unknown_keys_and_density_are_tolerated() {
        let t = Theme::from_toml(
            r##"
density = "ultra-dense"
some_future_token = 42

[colors]
brand_new_color = "#123456"
"##,
        )
        .unwrap();
        assert_eq!(t.density.multiplier(), 1.0);
    }

    #[test]
    fn invalid_toml_is_an_error_not_a_panic() {
        assert!(Theme::from_toml("= not toml [").is_err());
        assert!(Theme::from_toml("window_opacity = \"loud\"").is_err());
    }

    #[test]
    fn opacity_is_clamped() {
        let t = Theme::from_toml("window_opacity = 7.5").unwrap();
        assert_eq!(t.window_opacity, 1.0);
        let t = Theme::from_toml("window_opacity = -2.0").unwrap();
        assert_eq!(t.window_opacity, 0.0);
    }

    #[test]
    fn css_contains_tokens() {
        let t = Theme::default();
        let css = t.to_css();
        assert!(css.contains("@define-color vesper_bg_base #121318;"));
        assert!(css.contains("@define-color vesper_accent #7d8cff;"));
        assert!(css.contains("window.vesper-window"));
        assert!(css.contains(".vesper-message"));
        // Opaque theme: window uses the base background.
        assert!(css.contains("background: @vesper_bg_base"));
    }

    #[test]
    fn translucent_theme_gets_transparent_window() {
        let t = Theme::from_toml("window_opacity = 0.8").unwrap();
        assert!(t.to_css().contains("background: transparent"));
    }

    #[test]
    fn builtin_themes_parse() {
        for (name, text) in BUILTIN_THEMES {
            let t = Theme::from_toml(text)
                .unwrap_or_else(|e| panic!("builtin theme {name} failed: {e}"));
            if *name == "vesper-glass" {
                assert!(t.window_opacity < 1.0);
                assert!(t.blur);
            }
        }
    }

    #[test]
    fn override_css_is_appended() {
        let resolved = ResolvedTheme {
            name: "x".into(),
            theme: Theme::default(),
            override_css: Some(".vesper-message { color: red; }".into()),
        };
        assert!(resolved.css().ends_with(".vesper-message { color: red; }"));
    }

    #[test]
    fn resolve_prefers_user_theme_over_builtin() {
        let dir = std::env::temp_dir().join(format!("vesper-theme-test-{}", std::process::id()));
        let theme_dir = dir.join("vesper-dark");
        std::fs::create_dir_all(&theme_dir).unwrap();
        std::fs::write(theme_dir.join("theme.toml"), "spacing_unit = 99").unwrap();
        let resolved = resolve_theme(&dir, "vesper-dark").unwrap();
        assert_eq!(resolved.theme.spacing_unit, 99.0);
        // Built-in fallback still works for names without a user dir.
        let builtin = resolve_theme(&dir, "vesper-light").unwrap();
        assert_ne!(builtin.theme.spacing_unit, 99.0);
        // Missing themes are an error.
        assert!(resolve_theme(&dir, "no-such-theme").is_err());
        std::fs::remove_dir_all(&dir).ok();
    }
}
