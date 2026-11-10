//! Platform-specific integrations (KWin blur, global menu, voice, screen
//! share) live behind this trait. In this pass only no-op stubs exist; real
//! implementations land in later passes.

/// Desktop-environment specific features. Implementations must be safe to
/// call on any platform; unsupported features silently do nothing.
///
/// Only `apply_blur` is wired up today; the remaining methods are the stable
/// surface that later passes (global menu, voice, screen share) will call.
#[allow(dead_code)]
pub trait PlatformIntegration {
    /// Request compositor blur behind the window (e.g. via the KWin
    /// `_KDE_NET_WM_BLUR_BEHIND_REGION` protocol or the KWin D-Bus API).
    /// `strength` is the theme's `blur_strength` hint.
    fn apply_blur(&self, window: &gtk::Window, enabled: bool, strength: f64);

    /// Export the application menu over D-Bus for global-menu desktops
    /// (`com.canonical.AppMenu.Registrar`).
    fn setup_global_menu(&self, window: &gtk::Window);

    /// Join a voice channel. Voice is out of scope for this pass.
    fn start_voice(&self);

    /// Start a screen-share session (xdg-desktop-portal ScreenCast).
    /// Out of scope for this pass.
    fn start_screen_share(&self);
}

/// Stub used until real platform backends exist.
pub struct NoopPlatform;

impl PlatformIntegration for NoopPlatform {
    fn apply_blur(&self, _window: &gtk::Window, enabled: bool, strength: f64) {
        tracing::debug!(enabled, strength, "blur requested; no platform backend yet");
    }

    fn setup_global_menu(&self, _window: &gtk::Window) {
        tracing::debug!("global menu requested; no platform backend yet");
    }

    fn start_voice(&self) {
        tracing::debug!("voice requested; not implemented in this pass");
    }

    fn start_screen_share(&self) {
        tracing::debug!("screen share requested; not implemented in this pass");
    }
}

/// Returns the best platform integration for the current desktop.
/// Currently always the no-op stub.
pub fn detect() -> Box<dyn PlatformIntegration> {
    Box::new(NoopPlatform)
}
