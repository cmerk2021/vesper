//! Tolerant-parsing tests against realistic user-account fixtures.
//! These must never error on unknown fields or missing optional fields.

use vesper_core::models::{Channel, ChannelType, GatewayPayload, Guild, Message, Ready, Snowflake};

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/tests/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .expect("fixture file")
}

#[test]
fn ready_fixture_parses() {
    let ready: Ready = serde_json::from_str(&fixture("ready.json")).expect("READY must parse");
    let user = ready.user.expect("user");
    assert_eq!(user.id, Some(Snowflake(190320984123768832)));
    assert_eq!(user.display_name(), "Vesper User");
    assert_eq!(
        ready.session_id.as_deref(),
        Some("8d9e0f1a2b3c4d5e6f708192a3b4c5d6")
    );
    assert_eq!(
        ready.resume_gateway_url.as_deref(),
        Some("wss://gateway-us-east1-b.discord.gg")
    );

    assert_eq!(ready.guilds.len(), 1);
    let guild = &ready.guilds[0];
    // User-account READY nests name/icon under `properties`.
    assert_eq!(guild.display_name(), "Vesper Hangout");
    assert!(guild.icon_url(96).unwrap().contains("80351110224678912"));
    assert_eq!(guild.channels.len(), 3);
    assert_eq!(guild.channels[0].kind, ChannelType::GuildCategory);
    assert_eq!(guild.channels[1].kind, ChannelType::GuildText);
    assert_eq!(guild.channels[1].display_name(), "general");

    // private_channels carry recipient_ids for user accounts.
    assert_eq!(ready.private_channels.len(), 2);
    assert_eq!(ready.private_channels[0].kind, ChannelType::Dm);
    assert_eq!(
        ready.private_channels[0].recipient_ids.as_ref().unwrap()[0],
        Snowflake(82198898841029460)
    );
    assert_eq!(ready.users.len(), 1);
    assert!(ready.read_state.is_some());
}

#[test]
fn ready_inside_gateway_payload_parses() {
    let frame = format!(
        r#"{{"op":0,"s":1,"t":"READY","d":{}}}"#,
        fixture("ready.json")
    );
    let payload: GatewayPayload = serde_json::from_str(&frame).unwrap();
    assert_eq!(payload.op, 0);
    assert_eq!(payload.t.as_deref(), Some("READY"));
    let _: Ready = serde_json::from_value(payload.d.unwrap()).unwrap();
}

#[test]
fn guild_create_fixture_parses() {
    let guild: Guild =
        serde_json::from_str(&fixture("guild_create.json")).expect("GUILD_CREATE must parse");
    assert_eq!(guild.id, Snowflake(197038439483310086));
    assert_eq!(guild.display_name(), "Discord Testers");
    assert_eq!(guild.channels.len(), 3);
    // Channel type 15 (forum) is newer than our known set: preserved, not an error.
    assert_eq!(guild.channels[2].kind, ChannelType::Unknown(15));
}

#[test]
fn message_create_fixture_parses() {
    let msg: Message =
        serde_json::from_str(&fixture("message_create.json")).expect("MESSAGE_CREATE must parse");
    assert_eq!(msg.id, Snowflake(334385199974967042));
    assert_eq!(msg.channel_id, Snowflake(103735883630395393));
    assert_eq!(msg.author.as_ref().unwrap().display_name(), "Friend");
    assert!(msg.content.as_deref().unwrap().contains("example.com"));
    assert_eq!(msg.attachments.len(), 1);
    assert_eq!(
        msg.attachments[0].filename.as_deref(),
        Some("screenshot.png")
    );
    assert_eq!(msg.embeds.len(), 1);
    assert_eq!(
        msg.message_reference.unwrap().message_id,
        Some(Snowflake(334385199974967000))
    );
}

#[test]
fn malformed_message_still_parses() {
    // Numeric snowflakes, null optionals, unknown message type, unknown
    // fields everywhere, and attachments with no known fields.
    let msg: Message = serde_json::from_str(&fixture("malformed_message.json"))
        .expect("tolerant parsing must not error");
    assert_eq!(msg.id, Snowflake(334385199974967099));
    assert_eq!(msg.kind, Some(42));
    assert!(msg.author.is_none());
    assert!(msg.content.is_none());
    assert_eq!(msg.attachments.len(), 1);
    assert!(msg.attachments[0].filename.is_none());
    // Embeds stay raw even when their contents look odd.
    assert_eq!(msg.embeds.len(), 2);
}

#[test]
fn channel_with_missing_fields_parses() {
    let c: Channel = serde_json::from_str(r#"{"id":"5"}"#).unwrap();
    assert_eq!(c.id, Snowflake(5));
    assert!(c.name.is_none());
}
