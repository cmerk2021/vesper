//! Tolerant serde models for the Discord user-account API.
//!
//! Rules enforced here:
//! - Every field is `Option<T>` or `#[serde(default)]` so missing fields never
//!   cause a parse error.
//! - Unknown fields are ignored (serde's default behaviour).
//! - Unknown enum values map to catch-all variants.
//! - Snowflake IDs deserialize from either a JSON string or number.

use std::fmt;

use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A Discord snowflake ID. Deserializes from a JSON string or number and
/// serializes back to a string (what the API expects).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Snowflake(pub u64);

impl fmt::Display for Snowflake {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<u64> for Snowflake {
    fn from(v: u64) -> Self {
        Snowflake(v)
    }
}

impl Serialize for Snowflake {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0.to_string())
    }
}

struct SnowflakeVisitor;

impl Visitor<'_> for SnowflakeVisitor {
    type Value = Snowflake;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("a snowflake as a string or integer")
    }

    fn visit_u64<E: de::Error>(self, v: u64) -> Result<Snowflake, E> {
        Ok(Snowflake(v))
    }

    fn visit_i64<E: de::Error>(self, v: i64) -> Result<Snowflake, E> {
        Ok(Snowflake(v.max(0) as u64))
    }

    fn visit_str<E: de::Error>(self, v: &str) -> Result<Snowflake, E> {
        v.parse::<u64>()
            .map(Snowflake)
            .map_err(|_| E::custom("invalid snowflake string"))
    }
}

impl<'de> Deserialize<'de> for Snowflake {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(SnowflakeVisitor)
    }
}

/// A Discord user (partial; only fields Vesper needs).
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct User {
    pub id: Option<Snowflake>,
    pub username: Option<String>,
    pub discriminator: Option<String>,
    pub global_name: Option<String>,
    pub avatar: Option<String>,
    pub bot: Option<bool>,
}

impl User {
    /// Best display name available.
    pub fn display_name(&self) -> String {
        self.global_name
            .clone()
            .or_else(|| self.username.clone())
            .unwrap_or_else(|| "Unknown".to_string())
    }

    /// CDN URL for this user's avatar, if set.
    pub fn avatar_url(&self, size: u32) -> Option<String> {
        let id = self.id?;
        let hash = self.avatar.as_deref()?;
        Some(format!(
            "https://cdn.discordapp.com/avatars/{id}/{hash}.png?size={size}"
        ))
    }
}

/// Channel type. Unknown values are preserved, never an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelType {
    GuildText,
    Dm,
    GuildVoice,
    GroupDm,
    GuildCategory,
    GuildAnnouncement,
    Unknown(u64),
}

impl Default for ChannelType {
    fn default() -> Self {
        ChannelType::Unknown(u64::MAX)
    }
}

impl ChannelType {
    pub fn from_raw(v: u64) -> Self {
        match v {
            0 => ChannelType::GuildText,
            1 => ChannelType::Dm,
            2 => ChannelType::GuildVoice,
            3 => ChannelType::GroupDm,
            4 => ChannelType::GuildCategory,
            5 => ChannelType::GuildAnnouncement,
            other => ChannelType::Unknown(other),
        }
    }

    pub fn to_raw(self) -> u64 {
        match self {
            ChannelType::GuildText => 0,
            ChannelType::Dm => 1,
            ChannelType::GuildVoice => 2,
            ChannelType::GroupDm => 3,
            ChannelType::GuildCategory => 4,
            ChannelType::GuildAnnouncement => 5,
            ChannelType::Unknown(v) => v,
        }
    }
}

impl<'de> Deserialize<'de> for ChannelType {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let v = u64::deserialize(deserializer)?;
        Ok(ChannelType::from_raw(v))
    }
}

impl Serialize for ChannelType {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u64(self.to_raw())
    }
}

/// A guild or DM channel.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Channel {
    pub id: Snowflake,
    #[serde(rename = "type")]
    pub kind: ChannelType,
    pub guild_id: Option<Snowflake>,
    pub name: Option<String>,
    pub topic: Option<String>,
    pub position: Option<i64>,
    pub parent_id: Option<Snowflake>,
    pub last_message_id: Option<Snowflake>,
    pub recipients: Option<Vec<User>>,
    /// User READY private_channels often carry recipient IDs instead of
    /// full user objects.
    pub recipient_ids: Option<Vec<Snowflake>>,
}

impl Channel {
    /// Human-readable name for the channel list.
    pub fn display_name(&self) -> String {
        if let Some(name) = &self.name {
            if !name.is_empty() {
                return name.clone();
            }
        }
        if let Some(recipients) = &self.recipients {
            let names: Vec<String> = recipients.iter().map(|u| u.display_name()).collect();
            if !names.is_empty() {
                return names.join(", ");
            }
        }
        "unnamed".to_string()
    }
}

/// A guild (server).
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Guild {
    pub id: Snowflake,
    pub name: Option<String>,
    pub icon: Option<String>,
    pub channels: Vec<Channel>,
    /// User-account GUILD_CREATE nests most fields under `properties`.
    pub properties: Option<GuildProperties>,
    pub unavailable: Option<bool>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct GuildProperties {
    pub id: Option<Snowflake>,
    pub name: Option<String>,
    pub icon: Option<String>,
}

impl Guild {
    /// Guild name, looking through the user-account `properties` nesting.
    pub fn display_name(&self) -> String {
        self.name
            .clone()
            .or_else(|| self.properties.as_ref().and_then(|p| p.name.clone()))
            .unwrap_or_else(|| "Unknown server".to_string())
    }

    pub fn icon_hash(&self) -> Option<String> {
        self.icon
            .clone()
            .or_else(|| self.properties.as_ref().and_then(|p| p.icon.clone()))
    }

    /// CDN URL for the guild icon, if set.
    pub fn icon_url(&self, size: u32) -> Option<String> {
        let hash = self.icon_hash()?;
        Some(format!(
            "https://cdn.discordapp.com/icons/{}/{hash}.png?size={size}",
            self.id
        ))
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Attachment {
    pub id: Option<Snowflake>,
    pub filename: Option<String>,
    pub size: Option<u64>,
    pub url: Option<String>,
    pub proxy_url: Option<String>,
    pub content_type: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct MessageReference {
    pub message_id: Option<Snowflake>,
    pub channel_id: Option<Snowflake>,
    pub guild_id: Option<Snowflake>,
}

/// A chat message.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Message {
    pub id: Snowflake,
    pub channel_id: Snowflake,
    pub guild_id: Option<Snowflake>,
    pub author: Option<User>,
    pub content: Option<String>,
    /// ISO 8601 timestamp string; parsed by the UI layer.
    pub timestamp: Option<String>,
    pub edited_timestamp: Option<String>,
    #[serde(rename = "type")]
    pub kind: Option<u64>,
    pub attachments: Vec<Attachment>,
    /// Embeds are kept raw; rendering is out of scope for this pass.
    pub embeds: Vec<serde_json::Value>,
    pub message_reference: Option<MessageReference>,
    pub nonce: Option<serde_json::Value>,
}

/// Raw gateway frame: `{"op":0,"d":{...},"s":42,"t":"MESSAGE_CREATE"}`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GatewayPayload {
    pub op: u8,
    #[serde(default)]
    pub d: Option<serde_json::Value>,
    #[serde(default)]
    pub s: Option<u64>,
    #[serde(default)]
    pub t: Option<String>,
}

/// The parts of the user-account READY dispatch Vesper needs. The real
/// payload is enormous; everything else is ignored.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Ready {
    pub v: Option<u64>,
    pub user: Option<User>,
    pub session_id: Option<String>,
    pub resume_gateway_url: Option<String>,
    pub guilds: Vec<Guild>,
    pub private_channels: Vec<Channel>,
    pub users: Vec<User>,
    /// User accounts receive read_state in varying shapes; keep it raw.
    pub read_state: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Hello {
    pub heartbeat_interval: u64,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct TypingStart {
    pub channel_id: Snowflake,
    pub user_id: Snowflake,
    pub guild_id: Option<Snowflake>,
    pub timestamp: Option<u64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct MessageDelete {
    pub id: Snowflake,
    pub channel_id: Snowflake,
    pub guild_id: Option<Snowflake>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snowflake_from_string_and_number() {
        let s: Snowflake = serde_json::from_str("\"80351110224678912\"").unwrap();
        assert_eq!(s, Snowflake(80351110224678912));
        let n: Snowflake = serde_json::from_str("80351110224678912").unwrap();
        assert_eq!(n, Snowflake(80351110224678912));
    }

    #[test]
    fn snowflake_serializes_to_string() {
        assert_eq!(
            serde_json::to_string(&Snowflake(42)).unwrap(),
            "\"42\"".to_string()
        );
    }

    #[test]
    fn unknown_channel_type_is_preserved() {
        let c: Channel = serde_json::from_str(r#"{"id":"1","type":99}"#).unwrap();
        assert_eq!(c.kind, ChannelType::Unknown(99));
    }

    #[test]
    fn empty_objects_parse() {
        let _: User = serde_json::from_str("{}").unwrap();
        let _: Guild = serde_json::from_str("{}").unwrap();
        let _: Channel = serde_json::from_str("{}").unwrap();
        let _: Message = serde_json::from_str("{}").unwrap();
        let _: Ready = serde_json::from_str("{}").unwrap();
    }
}
