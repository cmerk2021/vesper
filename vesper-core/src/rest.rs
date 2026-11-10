//! REST client for the Discord user-account API.
//!
//! The `Authorization` header carries the raw user token (no `Bot ` prefix).
//! The token is never logged.

use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, USER_AGENT};
use reqwest::StatusCode;
use serde::Deserialize;
use tokio::time::{sleep, Duration};
use tracing::{debug, warn};

use crate::models::{Channel, Message, Snowflake, User};
use crate::props;
use crate::CoreError;

const API_BASE: &str = "https://discord.com/api/v10";
const MAX_RATE_LIMIT_RETRIES: u32 = 5;

#[derive(Clone)]
pub struct RestClient {
    http: reqwest::Client,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct RateLimited {
    retry_after: f64,
}

impl RestClient {
    pub fn new(token: &str) -> Result<Self, CoreError> {
        let mut headers = HeaderMap::new();
        let mut auth = HeaderValue::from_str(token.trim()).map_err(|_| CoreError::Unauthorized)?;
        auth.set_sensitive(true);
        headers.insert(AUTHORIZATION, auth);
        headers.insert(USER_AGENT, HeaderValue::from_static(props::USER_AGENT));
        headers.insert(
            "X-Super-Properties",
            HeaderValue::from_str(&props::super_properties_b64())
                .expect("base64 is always a valid header"),
        );
        let http = reqwest::Client::builder()
            .default_headers(headers)
            .build()?;
        Ok(RestClient { http })
    }

    /// Sends a request, retrying on 429 using `retry_after`.
    async fn execute(
        &self,
        build: impl Fn(&reqwest::Client) -> reqwest::RequestBuilder,
    ) -> Result<reqwest::Response, CoreError> {
        for _ in 0..MAX_RATE_LIMIT_RETRIES {
            let resp = build(&self.http).send().await?;
            match resp.status() {
                StatusCode::TOO_MANY_REQUESTS => {
                    let info: RateLimited = resp.json().await.unwrap_or_default();
                    let wait = Duration::from_secs_f64(info.retry_after.clamp(0.1, 120.0));
                    warn!(?wait, "rate limited (429); waiting");
                    sleep(wait).await;
                }
                StatusCode::UNAUTHORIZED => return Err(CoreError::Unauthorized),
                s if s.is_success() => return Ok(resp),
                s => {
                    debug!(status = %s, "unexpected REST status");
                    return Err(CoreError::UnexpectedStatus { status: s.as_u16() });
                }
            }
        }
        Err(CoreError::UnexpectedStatus { status: 429 })
    }

    /// GET /users/@me
    pub async fn get_current_user(&self) -> Result<User, CoreError> {
        let resp = self
            .execute(|c| c.get(format!("{API_BASE}/users/@me")))
            .await?;
        Ok(resp.json().await?)
    }

    /// GET /channels/{id}/messages?limit=N[&before=ID]
    ///
    /// Returned newest-first by Discord; callers usually reverse.
    pub async fn get_messages(
        &self,
        channel_id: Snowflake,
        limit: u8,
        before: Option<Snowflake>,
    ) -> Result<Vec<Message>, CoreError> {
        let resp = self
            .execute(|c| {
                let mut req = c
                    .get(format!("{API_BASE}/channels/{channel_id}/messages"))
                    .query(&[("limit", limit.to_string())]);
                if let Some(before) = before {
                    req = req.query(&[("before", before.to_string())]);
                }
                req
            })
            .await?;
        Ok(resp.json().await?)
    }

    /// POST /channels/{id}/messages. `reply_to` sets `message_reference`
    /// (the reply UI itself lands in pass 2).
    pub async fn send_message(
        &self,
        channel_id: Snowflake,
        content: &str,
        reply_to: Option<Snowflake>,
    ) -> Result<Message, CoreError> {
        let mut body = serde_json::json!({ "content": content });
        if let Some(reply) = reply_to {
            body["message_reference"] = serde_json::json!({
                "message_id": reply.to_string(),
                "channel_id": channel_id.to_string(),
            });
        }
        let resp = self
            .execute(|c| {
                c.post(format!("{API_BASE}/channels/{channel_id}/messages"))
                    .json(&body)
            })
            .await?;
        Ok(resp.json().await?)
    }

    /// GET /guilds/{id}/channels — fallback when GUILD_CREATE did not carry
    /// channel data (it usually does for user accounts).
    pub async fn get_guild_channels(&self, guild_id: Snowflake) -> Result<Vec<Channel>, CoreError> {
        let resp = self
            .execute(|c| c.get(format!("{API_BASE}/guilds/{guild_id}/channels")))
            .await?;
        Ok(resp.json().await?)
    }
}
