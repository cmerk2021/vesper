//! The core session: owns the tokio runtime on a background thread and
//! bridges the UI via a `Command` channel (UI -> core) and an `Event`
//! channel (core -> UI).

use std::collections::HashMap;

use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::models::{Channel, Guild, Message, Snowflake, User};
use crate::rest::RestClient;
use crate::{gateway, CoreError};

const KEYRING_ATTRS: [(&str, &str); 2] = [("application", "vesper"), ("type", "discord-token")];
const KEYRING_LABEL: &str = "Vesper Discord token";

/// Commands sent from the UI to the core.
#[derive(Debug, Clone)]
pub enum Command {
    /// Load a stored token (if any) and log in, otherwise ask for login.
    Init,
    /// Log in with an explicit token (from the webview or the paste field).
    Login { token: String },
    /// Delete the stored token and tear the session down.
    Logout,
    /// Fetch the latest messages for a channel.
    OpenChannel { channel_id: Snowflake },
    /// Send a message. `reply_to` populates `message_reference`.
    SendMessage {
        channel_id: Snowflake,
        content: String,
        reply_to: Option<Snowflake>,
    },
    /// Fetch channels for a guild over REST (fallback path).
    FetchGuildChannels { guild_id: Snowflake },
}

/// Events sent from the core to the UI.
#[derive(Debug, Clone)]
pub enum Event {
    /// Show the login screen, optionally with an error message.
    NeedsLogin {
        error: Option<String>,
    },
    /// A login attempt is in progress.
    LoggingIn,
    /// The gateway sent READY.
    Ready {
        user: User,
        guilds: Vec<Guild>,
        private_channels: Vec<Channel>,
        users: Vec<User>,
    },
    GatewayConnected,
    GatewayDisconnected {
        reconnecting: bool,
    },
    GuildCreate(Box<Guild>),
    ChannelCreate(Box<Channel>),
    ChannelUpdate(Box<Channel>),
    ChannelDelete(Box<Channel>),
    MessageCreate(Box<Message>),
    MessageUpdate(Box<Message>),
    MessageDelete {
        channel_id: Snowflake,
        id: Snowflake,
    },
    TypingStart {
        channel_id: Snowflake,
        user_id: Snowflake,
    },
    /// Result of `Command::OpenChannel`, oldest message first.
    MessagesLoaded {
        channel_id: Snowflake,
        messages: Vec<Message>,
    },
    /// Result of `Command::FetchGuildChannels`.
    GuildChannelsLoaded {
        guild_id: Snowflake,
        channels: Vec<Channel>,
    },
    LoggedOut,
    /// Non-fatal error the UI may surface.
    Error {
        message: String,
    },
}

/// Handle held by the UI: a command sender plus an event receiver that can be
/// consumed on the GTK main loop.
pub struct CoreHandle {
    pub commands: CommandSender,
    pub events: async_channel::Receiver<Event>,
}

/// Cloneable sender for UI widgets that need to issue commands.
pub type CommandSender = mpsc::UnboundedSender<Command>;

impl CoreHandle {
    pub fn send(&self, cmd: Command) {
        if self.commands.send(cmd).is_err() {
            warn!("core command channel closed");
        }
    }
}

/// Spawns the core on a background thread with its own tokio runtime.
pub fn spawn() -> CoreHandle {
    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
    let (event_tx, event_rx) = async_channel::unbounded();

    std::thread::Builder::new()
        .name("vesper-core".to_string())
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("failed to build tokio runtime");
            rt.block_on(run(cmd_rx, event_tx));
        })
        .expect("failed to spawn core thread");

    CoreHandle {
        commands: cmd_tx,
        events: event_rx,
    }
}

struct Session {
    rest: RestClient,
    gateway_task: tokio::task::JoinHandle<()>,
}

async fn run(mut commands: mpsc::UnboundedReceiver<Command>, events: async_channel::Sender<Event>) {
    let mut session: Option<Session> = None;

    while let Some(cmd) = commands.recv().await {
        match cmd {
            Command::Init => {
                let _ = events.send(Event::LoggingIn).await;
                match load_token().await {
                    Ok(Some(token)) => start_session(&token, &mut session, &events, false).await,
                    Ok(None) => {
                        let _ = events.send(Event::NeedsLogin { error: None }).await;
                    }
                    Err(e) => {
                        warn!("keyring unavailable: {e}");
                        let _ = events.send(Event::NeedsLogin { error: None }).await;
                    }
                }
            }
            Command::Login { token } => {
                let _ = events.send(Event::LoggingIn).await;
                start_session(token.trim(), &mut session, &events, true).await;
            }
            Command::Logout => {
                if let Some(s) = session.take() {
                    s.gateway_task.abort();
                }
                if let Err(e) = delete_token().await {
                    warn!("failed to delete stored token: {e}");
                }
                let _ = events.send(Event::LoggedOut).await;
                let _ = events.send(Event::NeedsLogin { error: None }).await;
            }
            Command::OpenChannel { channel_id } => {
                if let Some(s) = &session {
                    match s.rest.get_messages(channel_id, 50, None).await {
                        Ok(mut messages) => {
                            messages.reverse(); // Discord returns newest first.
                            let _ = events
                                .send(Event::MessagesLoaded {
                                    channel_id,
                                    messages,
                                })
                                .await;
                        }
                        Err(e) => {
                            let _ = events
                                .send(Event::Error {
                                    message: format!("Failed to load messages: {e}"),
                                })
                                .await;
                        }
                    }
                }
            }
            Command::SendMessage {
                channel_id,
                content,
                reply_to,
            } => {
                if let Some(s) = &session {
                    if let Err(e) = s.rest.send_message(channel_id, &content, reply_to).await {
                        let _ = events
                            .send(Event::Error {
                                message: format!("Failed to send message: {e}"),
                            })
                            .await;
                    }
                }
            }
            Command::FetchGuildChannels { guild_id } => {
                if let Some(s) = &session {
                    match s.rest.get_guild_channels(guild_id).await {
                        Ok(channels) => {
                            let _ = events
                                .send(Event::GuildChannelsLoaded { guild_id, channels })
                                .await;
                        }
                        Err(e) => {
                            let _ = events
                                .send(Event::Error {
                                    message: format!("Failed to load channels: {e}"),
                                })
                                .await;
                        }
                    }
                }
            }
        }
    }
}

/// Validates the token over REST, persists it, and starts the gateway.
async fn start_session(
    token: &str,
    session: &mut Option<Session>,
    events: &async_channel::Sender<Event>,
    store: bool,
) {
    if let Some(old) = session.take() {
        old.gateway_task.abort();
    }

    let rest = match RestClient::new(token) {
        Ok(r) => r,
        Err(_) => {
            let _ = events
                .send(Event::NeedsLogin {
                    error: Some("That does not look like a valid token.".to_string()),
                })
                .await;
            return;
        }
    };

    match rest.get_current_user().await {
        Ok(user) => {
            info!("logged in as {}", user.display_name());
        }
        Err(CoreError::Unauthorized) => {
            // Invalid token: drop any stored copy and go back to login.
            let _ = delete_token().await;
            let _ = events
                .send(Event::NeedsLogin {
                    error: Some(
                        "Discord rejected the token (401). Please log in again.".to_string(),
                    ),
                })
                .await;
            return;
        }
        Err(e) => {
            let _ = events
                .send(Event::NeedsLogin {
                    error: Some(format!("Could not reach Discord: {e}")),
                })
                .await;
            return;
        }
    }

    if store {
        if let Err(e) = store_token(token).await {
            warn!("failed to store token in keyring: {e}");
            let _ = events
                .send(Event::Error {
                    message: "Could not store the token in the keyring; you will need to log in again next time.".to_string(),
                })
                .await;
        }
    }

    let gateway_task = tokio::spawn(gateway::run_gateway(token.to_string(), events.clone()));
    *session = Some(Session { rest, gateway_task });
}

fn keyring_attrs() -> HashMap<&'static str, &'static str> {
    KEYRING_ATTRS.into_iter().collect()
}

async fn load_token() -> Result<Option<String>, CoreError> {
    let keyring = oo7::Keyring::new()
        .await
        .map_err(|e| CoreError::Keyring(e.to_string()))?;
    let items = keyring
        .search_items(&keyring_attrs())
        .await
        .map_err(|e| CoreError::Keyring(e.to_string()))?;
    if let Some(item) = items.first() {
        let secret = item
            .secret()
            .await
            .map_err(|e| CoreError::Keyring(e.to_string()))?;
        let token = String::from_utf8_lossy(&secret).trim().to_string();
        if !token.is_empty() {
            return Ok(Some(token));
        }
    }
    Ok(None)
}

async fn store_token(token: &str) -> Result<(), CoreError> {
    let keyring = oo7::Keyring::new()
        .await
        .map_err(|e| CoreError::Keyring(e.to_string()))?;
    keyring
        .create_item(KEYRING_LABEL, &keyring_attrs(), token.as_bytes(), true)
        .await
        .map_err(|e| CoreError::Keyring(e.to_string()))?;
    Ok(())
}

async fn delete_token() -> Result<(), CoreError> {
    let keyring = oo7::Keyring::new()
        .await
        .map_err(|e| CoreError::Keyring(e.to_string()))?;
    keyring
        .delete(&keyring_attrs())
        .await
        .map_err(|e| CoreError::Keyring(e.to_string()))?;
    Ok(())
}
