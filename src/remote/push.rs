//! Reaching the phone when an agent stops for you.
//!
//! Everything else here is pull: the page asks what is happening, and it only
//! asks while you are looking at it. That leaves the feature's own premise
//! unmet — an agent that blocks while you are away is dead time until you
//! think to check.
//!
//! Web Push closes it. The phone subscribes through its browser's push
//! service; workbench signs a request to that service with a VAPID keypair it
//! generates once; the notification arrives whether or not the page is open,
//! and whether or not you are on the tailnet.
//!
//! ```text
//! phone  ──subscribe──▶ workbench          (endpoint, stored)
//! agent blocks ──▶ workbench ──POST──▶ push service ──▶ phone ──▶ sw.js
//! ```
//!
//! **The notification carries no payload.** Encrypting one means ECDH against
//! the subscription's key, HKDF, and AES-GCM — a lot of cryptography to get
//! right against a service that cannot be tested from here. A bare poke is
//! plain VAPID, and the service worker then reads `/api/state` and writes the
//! text from what is *true at delivery*, which is better than a snapshot of
//! what was true when the agent stopped. It falls back to a generic line when
//! the phone is off the tailnet and cannot read anything.

use anyhow::{anyhow, Result};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use p256::ecdsa::{signature::Signer, Signature, SigningKey};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::process::{Command, Stdio};

/// How long a signed request stays valid. Push services reject anything more
/// than 24 hours out; this is signed per send, so it only has to outlive the
/// request.
const JWT_TTL_SECONDS: i64 = 60 * 60;

/// Who is sending, which every push service wants a way to reach.
///
/// It has to be a URI a push service will accept, and Apple checks: a
/// `mailto:` at `localhost` is refused outright with `BadJwtToken`, which is
/// a 403 on every notification and no clue on the phone. Probed against the
/// real service — `mailto:…@localhost` 403s where this, a real address, and
/// `…@example.com` all return 201.
const CONTACT: &str = "https://github.com/steferic/workbench";
/// How long the push service should hold the message for a phone that is off.
const TTL_SECONDS: u32 = 3600;

/// One device that asked to hear about blocked agents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Subscription {
    /// The push service URL to POST to. Opaque, and issued per device.
    pub endpoint: String,
}

/// The keypair identifying this workbench to push services, and the devices
/// listening. Persisted, because a new keypair invalidates every existing
/// subscription.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Push {
    /// The VAPID private scalar, base64url. Empty until first use.
    #[serde(default)]
    key: String,
    #[serde(default)]
    subscriptions: Vec<Subscription>,
}

impl Push {
    pub fn path() -> Result<PathBuf> {
        // Beside the other cross-process state, not in the hand-editable
        // config: a keypair and a list of opaque endpoints are not settings.
        Ok(crate::comms::comms_root()?.join("push.json"))
    }

    /// Load the stored keypair and subscriptions, minting a keypair the first
    /// time. Failure is not fatal anywhere: push is an extra.
    pub fn load() -> Push {
        let mut push: Push = Push::path()
            .ok()
            .and_then(|path| std::fs::read(path).ok())
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        if push.key.is_empty() {
            push.key =
                URL_SAFE_NO_PAD.encode(SigningKey::random(&mut rand::rngs::OsRng).to_bytes());
            if let Err(err) = push.save() {
                crate::logger::warn(format!("could not store the push keypair: {err}"));
            }
        }
        push
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        crate::comms::write_atomic(&path, serde_json::to_string_pretty(self)?.as_bytes())
    }

    fn signing_key(&self) -> Result<SigningKey> {
        let bytes = URL_SAFE_NO_PAD.decode(&self.key)?;
        SigningKey::from_slice(&bytes).map_err(|err| anyhow!("bad push key: {err}"))
    }

    /// The `applicationServerKey` the browser needs to subscribe: the public
    /// point, uncompressed, base64url.
    pub fn public_key(&self) -> String {
        match self.signing_key() {
            Ok(key) => {
                URL_SAFE_NO_PAD.encode(key.verifying_key().to_encoded_point(false).as_bytes())
            }
            Err(_) => String::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.subscriptions.is_empty()
    }

    /// Remember a device. Re-subscribing with the same endpoint is a no-op,
    /// which is what a page that registers on every load does.
    pub fn subscribe(&mut self, endpoint: String) -> bool {
        if endpoint.is_empty() || self.subscriptions.iter().any(|s| s.endpoint == endpoint) {
            return false;
        }
        self.subscriptions.push(Subscription { endpoint });
        true
    }

    /// Poke every subscribed device.
    ///
    /// Runs off the event loop: a push service is a network round trip, and
    /// the loop it would block is the one drawing the TUI.
    pub fn notify(&self) {
        for subscription in &self.subscriptions {
            let endpoint = subscription.endpoint.clone();
            let Ok(token) = self.authorization(&endpoint) else {
                continue;
            };
            std::thread::spawn(move || send(&endpoint, &token));
        }
    }

    /// The `Authorization` header for one endpoint: a JWT naming that service
    /// as its audience, plus the public key it should check the signature with.
    fn authorization(&self, endpoint: &str) -> Result<String> {
        let audience = origin_of(endpoint)?;
        let expiry = chrono::Utc::now().timestamp() + JWT_TTL_SECONDS;

        let header = URL_SAFE_NO_PAD.encode(br#"{"typ":"JWT","alg":"ES256"}"#);
        let claims = URL_SAFE_NO_PAD.encode(
            serde_json::json!({
                "aud": audience,
                "exp": expiry,
                "sub": CONTACT,
            })
            .to_string(),
        );
        let signed = format!("{header}.{claims}");
        let signature: Signature = self.signing_key()?.sign(signed.as_bytes());
        let jwt = format!("{signed}.{}", URL_SAFE_NO_PAD.encode(signature.to_bytes()));

        Ok(format!("vapid t={jwt}, k={}", self.public_key()))
    }
}

/// `https://web.push.apple.com/abc123` → `https://web.push.apple.com`.
///
/// The audience claim names the service, not the subscription — signing for
/// the full endpoint is rejected.
fn origin_of(endpoint: &str) -> Result<String> {
    let (scheme, rest) = endpoint
        .split_once("://")
        .ok_or_else(|| anyhow!("endpoint is not a url: {endpoint}"))?;
    let host = rest.split('/').next().unwrap_or_default();
    if host.is_empty() {
        return Err(anyhow!("endpoint has no host: {endpoint}"));
    }
    Ok(format!("{scheme}://{host}"))
}

/// POST the empty push.
///
/// Through `curl` rather than a Rust client: this is the only outbound TLS in
/// workbench, and a whole TLS stack in the dependency tree to make one request
/// an hour is a poor trade on a machine that ships curl.
fn send(endpoint: &str, authorization: &str) {
    let output = Command::new("curl")
        .args([
            "--silent",
            "--show-error",
            "--max-time",
            "20",
            "--write-out",
            "%{http_code}",
            "-X",
            "POST",
            "-H",
        ])
        .arg(format!("Authorization: {authorization}"))
        .args(["-H", &format!("TTL: {TTL_SECONDS}")])
        .args(["-H", "Urgency: high", "-H", "Content-Length: 0"])
        .arg(endpoint)
        .stdin(Stdio::null())
        .output();

    match output {
        Ok(output) => {
            let code = String::from_utf8_lossy(&output.stdout);
            let code = code.trim();
            // 201 is the success every service returns; 404/410 mean the
            // device unsubscribed or was wiped, and it will simply keep
            // failing until it subscribes again.
            if !code.starts_with('2') {
                crate::logger::warn(format!(
                    "push to {} refused: {code} {}",
                    origin_of(endpoint).unwrap_or_default(),
                    String::from_utf8_lossy(&output.stderr).trim()
                ));
            }
        }
        Err(err) => crate::logger::warn(format!("could not run curl to push: {err}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keyed() -> Push {
        Push {
            key: URL_SAFE_NO_PAD.encode(SigningKey::random(&mut rand::rngs::OsRng).to_bytes()),
            subscriptions: Vec::new(),
        }
    }

    #[test]
    fn the_public_key_is_the_uncompressed_point_a_browser_expects() {
        let key = keyed().public_key();
        let bytes = URL_SAFE_NO_PAD.decode(&key).unwrap();
        assert_eq!(bytes.len(), 65, "65 bytes: 0x04 then x and y");
        assert_eq!(bytes[0], 4);
        // It travels in a URL and in JSON, so it must survive both untouched.
        assert!(!key.contains('+') && !key.contains('/') && !key.contains('='));
    }

    #[test]
    fn the_audience_is_the_service_not_the_subscription() {
        assert_eq!(
            origin_of("https://web.push.apple.com/abc/def?x=1").unwrap(),
            "https://web.push.apple.com"
        );
        assert!(origin_of("not-a-url").is_err());
        assert!(origin_of("https://").is_err());
    }

    #[test]
    fn the_authorization_is_a_signed_jwt_the_service_can_check() {
        let push = keyed();
        let header = push
            .authorization("https://web.push.apple.com/abc")
            .unwrap();

        let (scheme, rest) = header.split_once(' ').unwrap();
        assert_eq!(scheme, "vapid");
        let (t, k) = rest.split_once(", ").unwrap();
        let jwt = t.strip_prefix("t=").unwrap();
        assert_eq!(k.strip_prefix("k=").unwrap(), push.public_key());

        let parts: Vec<&str> = jwt.split('.').collect();
        assert_eq!(parts.len(), 3);
        let claims: serde_json::Value =
            serde_json::from_slice(&URL_SAFE_NO_PAD.decode(parts[1]).unwrap()).unwrap();
        assert_eq!(claims["aud"], "https://web.push.apple.com");
        assert_eq!(claims["sub"], CONTACT);
        assert!(claims["exp"].as_i64().unwrap() > chrono::Utc::now().timestamp());
        // ES256 is a fixed-width r‖s pair, not a DER blob.
        assert_eq!(URL_SAFE_NO_PAD.decode(parts[2]).unwrap().len(), 64);

        // And it verifies against the key we published.
        use p256::ecdsa::signature::Verifier;
        let signed = format!("{}.{}", parts[0], parts[1]);
        let signature = Signature::from_slice(&URL_SAFE_NO_PAD.decode(parts[2]).unwrap()).unwrap();
        let verifying = push.signing_key().unwrap().verifying_key().to_owned();
        assert!(verifying.verify(signed.as_bytes(), &signature).is_ok());
    }

    /// The failure this guards is silent: Apple answers `BadJwtToken` with a
    /// 403 and nothing reaches the phone, so a contact it will not accept
    /// looks exactly like notifications being switched off.
    #[test]
    fn the_contact_is_one_a_push_service_will_accept() {
        let (scheme, rest) = CONTACT.split_once(':').expect("a URI with a scheme");
        assert!(
            matches!(scheme, "mailto" | "https"),
            "{scheme} is not a contact"
        );
        assert!(
            !rest.contains("localhost") && !rest.contains("127.0.0.1"),
            "a push service cannot reach {rest}, and Apple rejects it outright"
        );
    }

    #[test]
    fn a_device_is_remembered_once_however_often_it_asks() {
        let mut push = keyed();
        assert!(push.subscribe("https://push/1".into()));
        assert!(!push.subscribe("https://push/1".into()), "already known");
        assert!(push.subscribe("https://push/2".into()));
        assert!(!push.subscribe(String::new()), "nothing to send to");
        assert_eq!(push.subscriptions.len(), 2);
    }
}
