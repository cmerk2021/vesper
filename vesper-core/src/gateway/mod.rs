//! Discord gateway v10 client (JSON encoding, no compression).

pub mod backoff;

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio::time::{sleep, Instant};
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};
use tracing::{debug, info, warn};

use crate::models::{
    Channel, GatewayPayload, Guild, Hello, Message, MessageDelete, Ready, TypingStart,
};
use crate::props;
use crate::session::Event;
use backoff::{policy_for_close_code, Backoff, HeartbeatTracker, ReconnectPolicy};

const DEFAULT_GATEWAY_URL: &str = "wss://gateway.discord.gg/?v=10&encoding=json";

type WsStream = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

#[derive(Debug, Clone)]
struct ResumeInfo {
    session_id: String,
    seq: u64,
    url: String,
}

/// Outcome of a single gateway connection.
enum Outcome {
    /// Try again; resume if `Some`.
    Reconnect(Option<ResumeInfo>),
    /// Stop permanently (auth failure etc.).
    Fatal(String),
}

/// Runs the gateway with automatic resume/reconnect until the task is
/// aborted or a fatal error occurs.
pub(crate) async fn run_gateway(token: String, events: async_channel::Sender<Event>) {
    let mut backoff = Backoff::new(Duration::from_secs(1), Duration::from_secs(64));
    let mut resume: Option<ResumeInfo> = None;

    loop {
        match connect_once(&token, resume.clone(), &events).await {
            Ok(Outcome::Reconnect(next)) => {
                // Reaching Outcome::Reconnect means the connection got past
                // HELLO, so the backoff can start over.
                backoff.reset();
                let resumable = next.is_some();
                resume = next;
                let _ = events
                    .send(Event::GatewayDisconnected { reconnecting: true })
                    .await;
                let delay = backoff.next_delay_jittered();
                info!(?delay, resumable, "gateway disconnected; reconnecting");
                sleep(delay).await;
            }
            Ok(Outcome::Fatal(message)) => {
                warn!("gateway fatal: {message}");
                let _ = events
                    .send(Event::NeedsLogin {
                        error: Some(message),
                    })
                    .await;
                return;
            }
            Err(e) => {
                let delay = backoff.next_delay_jittered();
                warn!("gateway connection error: {e}; retrying in {delay:?}");
                let _ = events
                    .send(Event::GatewayDisconnected { reconnecting: true })
                    .await;
                sleep(delay).await;
            }
        }
    }
}

struct ConnState {
    seq: Option<u64>,
    session_id: Option<String>,
    resume_url: Option<String>,
}

async fn connect_once(
    token: &str,
    resume: Option<ResumeInfo>,
    events: &async_channel::Sender<Event>,
) -> Result<Outcome, crate::CoreError> {
    let url = resume
        .as_ref()
        .map(|r| normalize_gateway_url(&r.url))
        .unwrap_or_else(|| DEFAULT_GATEWAY_URL.to_string());
    debug!("connecting to gateway");
    let (mut ws, _resp) = connect_async(&url).await.map_err(Box::new)?;

    // Expect HELLO first.
    let hello = loop {
        match next_payload(&mut ws).await? {
            Some(p) if p.op == 10 => {
                let hello: Hello = serde_json::from_value(p.d.unwrap_or(serde_json::Value::Null))
                    .unwrap_or_default();
                break hello;
            }
            Some(p) => debug!(op = p.op, "ignoring pre-HELLO payload"),
            None => return Ok(Outcome::Reconnect(resume)),
        }
    };
    let interval_ms = if hello.heartbeat_interval == 0 {
        41_250
    } else {
        hello.heartbeat_interval
    };
    let mut heartbeat = HeartbeatTracker::new(interval_ms);

    let mut state = ConnState {
        seq: resume.as_ref().map(|r| r.seq),
        session_id: resume.as_ref().map(|r| r.session_id.clone()),
        resume_url: resume.as_ref().map(|r| r.url.clone()),
    };

    // IDENTIFY or RESUME.
    if let Some(r) = &resume {
        info!("resuming gateway session");
        let payload = serde_json::json!({
            "op": 6,
            "d": { "token": token, "session_id": r.session_id, "seq": r.seq }
        });
        ws.send(WsMessage::Text(payload.to_string().into()))
            .await
            .map_err(Box::new)?;
    } else {
        info!("identifying to gateway");
        let payload = serde_json::json!({
            "op": 2,
            "d": {
                "token": token,
                "capabilities": 0,
                "properties": props::super_properties(),
                "presence": { "status": "online", "since": 0, "activities": [], "afk": false },
                "compress": false,
            }
        });
        ws.send(WsMessage::Text(payload.to_string().into()))
            .await
            .map_err(Box::new)?;
    }

    let _ = events.send(Event::GatewayConnected).await;

    // First heartbeat after `interval * jitter`, then every interval.
    let first_beat = Instant::now() + heartbeat.first_delay(rand::random::<f64>());
    let mut ticker = tokio::time::interval_at(first_beat, heartbeat.interval());
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            _ = ticker.tick() => {
                if !heartbeat.on_send() {
                    warn!("missed heartbeat ACK; reconnecting to resume");
                    let _ = ws.close(None).await;
                    return Ok(Outcome::Reconnect(make_resume(&state)));
                }
                let beat = serde_json::json!({ "op": 1, "d": state.seq });
                ws.send(WsMessage::Text(beat.to_string().into())).await.map_err(Box::new)?;
            }
            msg = ws.next() => {
                let msg = match msg {
                    Some(Ok(m)) => m,
                    Some(Err(e)) => {
                        warn!("websocket read error: {e}");
                        return Ok(Outcome::Reconnect(make_resume(&state)));
                    }
                    None => return Ok(Outcome::Reconnect(make_resume(&state))),
                };
                match msg {
                    WsMessage::Text(text) => {
                        let payload: GatewayPayload = match serde_json::from_str(&text) {
                            Ok(p) => p,
                            Err(e) => { debug!("unparseable gateway frame ignored: {e}"); continue; }
                        };
                        if let Some(s) = payload.s {
                            state.seq = Some(s);
                        }
                        match payload.op {
                            0 => handle_dispatch(&payload, &mut state, events).await,
                            1 => {
                                let beat = serde_json::json!({ "op": 1, "d": state.seq });
                                ws.send(WsMessage::Text(beat.to_string().into())).await.map_err(Box::new)?;
                            }
                            7 => {
                                info!("gateway requested reconnect");
                                let _ = ws.close(None).await;
                                return Ok(Outcome::Reconnect(make_resume(&state)));
                            }
                            9 => {
                                let resumable = payload.d.as_ref().and_then(|d| d.as_bool()).unwrap_or(false);
                                info!(resumable, "gateway session invalidated");
                                let _ = ws.close(None).await;
                                // Per docs, wait a short random time before re-identifying.
                                sleep(Duration::from_millis(1_000 + (rand::random::<u64>() % 4_000))).await;
                                return Ok(Outcome::Reconnect(if resumable { make_resume(&state) } else { None }));
                            }
                            11 => heartbeat.on_ack(),
                            op => debug!(op, "ignoring unknown gateway op"),
                        }
                    }
                    WsMessage::Close(frame) => {
                        let code: u16 = frame.as_ref().map(|f| f.code.into()).unwrap_or_else(|| CloseCode::Abnormal.into());
                        info!(code, "gateway closed");
                        return Ok(match policy_for_close_code(code) {
                            ReconnectPolicy::Resume => Outcome::Reconnect(make_resume(&state)),
                            ReconnectPolicy::Reidentify => Outcome::Reconnect(None),
                            ReconnectPolicy::Fatal => Outcome::Fatal(format!(
                                "Discord closed the connection (code {code}). Your token may be invalid."
                            )),
                        });
                    }
                    WsMessage::Ping(_) | WsMessage::Pong(_) | WsMessage::Binary(_) | WsMessage::Frame(_) => {}
                }
            }
        }
    }
}

fn make_resume(state: &ConnState) -> Option<ResumeInfo> {
    Some(ResumeInfo {
        session_id: state.session_id.clone()?,
        seq: state.seq?,
        url: state
            .resume_url
            .clone()
            .unwrap_or_else(|| DEFAULT_GATEWAY_URL.to_string()),
    })
}

fn normalize_gateway_url(url: &str) -> String {
    if url.contains('?') {
        url.to_string()
    } else {
        format!("{}/?v=10&encoding=json", url.trim_end_matches('/'))
    }
}

async fn next_payload(ws: &mut WsStream) -> Result<Option<GatewayPayload>, crate::CoreError> {
    loop {
        match ws.next().await {
            Some(Ok(WsMessage::Text(text))) => match serde_json::from_str(&text) {
                Ok(p) => return Ok(Some(p)),
                Err(e) => debug!("unparseable gateway frame ignored: {e}"),
            },
            Some(Ok(WsMessage::Close(_))) | None => return Ok(None),
            Some(Ok(_)) => {}
            Some(Err(e)) => return Err(Box::new(e).into()),
        }
    }
}

/// Handles an op-0 dispatch. Parse failures and unknown events are logged
/// and ignored; they are never an error.
async fn handle_dispatch(
    payload: &GatewayPayload,
    state: &mut ConnState,
    events: &async_channel::Sender<Event>,
) {
    let t = payload.t.as_deref().unwrap_or("");
    let d = payload.d.clone().unwrap_or(serde_json::Value::Null);
    match t {
        "READY" => match serde_json::from_value::<Ready>(d) {
            Ok(ready) => {
                state.session_id = ready.session_id.clone();
                state.resume_url = ready.resume_gateway_url.clone();
                info!(
                    guilds = ready.guilds.len(),
                    dms = ready.private_channels.len(),
                    "gateway READY"
                );
                let _ = events
                    .send(Event::Ready {
                        user: ready.user.unwrap_or_default(),
                        guilds: ready.guilds,
                        private_channels: ready.private_channels,
                        users: ready.users,
                    })
                    .await;
            }
            Err(e) => warn!("failed to parse READY (ignored): {e}"),
        },
        "READY_SUPPLEMENTAL" => debug!("ignoring READY_SUPPLEMENTAL"),
        "RESUMED" => info!("gateway session resumed"),
        "GUILD_CREATE" => {
            dispatch_parsed::<Guild>(d, events, |g| Event::GuildCreate(Box::new(g))).await
        }
        "CHANNEL_CREATE" => {
            dispatch_parsed::<Channel>(d, events, |c| Event::ChannelCreate(Box::new(c))).await
        }
        "CHANNEL_UPDATE" => {
            dispatch_parsed::<Channel>(d, events, |c| Event::ChannelUpdate(Box::new(c))).await
        }
        "CHANNEL_DELETE" => {
            dispatch_parsed::<Channel>(d, events, |c| Event::ChannelDelete(Box::new(c))).await
        }
        "MESSAGE_CREATE" => {
            dispatch_parsed::<Message>(d, events, |m| Event::MessageCreate(Box::new(m))).await
        }
        "MESSAGE_UPDATE" => {
            dispatch_parsed::<Message>(d, events, |m| Event::MessageUpdate(Box::new(m))).await
        }
        "MESSAGE_DELETE" => {
            dispatch_parsed::<MessageDelete>(d, events, |m| Event::MessageDelete {
                channel_id: m.channel_id,
                id: m.id,
            })
            .await
        }
        "TYPING_START" => {
            dispatch_parsed::<TypingStart>(d, events, |ts| Event::TypingStart {
                channel_id: ts.channel_id,
                user_id: ts.user_id,
            })
            .await
        }
        other => debug!(event = other, "ignoring unknown gateway event"),
    }
}

async fn dispatch_parsed<T: serde::de::DeserializeOwned>(
    d: serde_json::Value,
    events: &async_channel::Sender<Event>,
    map: impl FnOnce(T) -> Event,
) {
    match serde_json::from_value::<T>(d) {
        Ok(v) => {
            let _ = events.send(map(v)).await;
        }
        Err(e) => warn!("failed to parse gateway event (ignored): {e}"),
    }
}
