//! Client identification shared by the gateway IDENTIFY payload and the REST
//! `X-Super-Properties` header. Vesper presents itself as a desktop browser
//! client on Linux.

use base64::Engine;

pub const USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 \
(KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

/// The `properties` object sent in IDENTIFY and (base64-encoded) in the
/// `X-Super-Properties` REST header.
pub fn super_properties() -> serde_json::Value {
    serde_json::json!({
        "os": "Linux",
        "browser": "Chrome",
        "device": "",
        "system_locale": "en-US",
        "browser_user_agent": USER_AGENT,
        "browser_version": "131.0.0.0",
        "os_version": "",
        "referrer": "",
        "referring_domain": "",
        "release_channel": "stable",
        "client_build_number": 350_000,
        "client_event_source": null,
    })
}

/// Base64-encoded JSON for the `X-Super-Properties` header.
pub fn super_properties_b64() -> String {
    base64::engine::general_purpose::STANDARD.encode(super_properties().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn super_properties_shape() {
        let v = super_properties();
        assert_eq!(v["os"], "Linux");
        assert_eq!(v["browser"], "Chrome");
        assert_eq!(v["device"], "");
    }

    #[test]
    fn super_properties_b64_roundtrips() {
        let b64 = super_properties_b64();
        let raw = base64::engine::general_purpose::STANDARD
            .decode(b64)
            .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&raw).unwrap();
        assert_eq!(v["os"], "Linux");
    }
}
