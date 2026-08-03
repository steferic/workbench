//! The HTTP server thread and its routes.
//!
//! Bound to the Tailscale address only. That is the security boundary that
//! matters — a café network cannot reach it even with the firewall off — and
//! the token is the second one, for the case of another device on your own
//! tailnet. Nothing here is ever exposed with `tailscale funnel`.

use anyhow::{anyhow, Result};
use std::net::{IpAddr, SocketAddr};
use std::process::Command;
use tiny_http::{Header, Response, Server};
use tokio::sync::mpsc;

use super::{page, Shared};
use crate::app::Action;

/// Where the server listens, and the token it demands.
#[derive(Debug, Clone)]
pub struct RemoteConfig {
    pub addr: SocketAddr,
    pub token: String,
}

impl RemoteConfig {
    /// The URL to put on your phone's home screen.
    pub fn url(&self) -> String {
        format!("http://{}/?t={}", self.addr, self.token)
    }
}

/// A running server, kept alive by holding this.
pub struct Remote {
    pub config: RemoteConfig,
}

/// What the phone asked for, resolved against app state by the event loop.
///
/// The server never touches `AppState`; it sends one of these and the loop
/// applies it, so a request cannot race the UI.
#[derive(Debug, Clone)]
pub enum RemoteCommand {
    /// Queue work for an agent.
    Todo { agent: String, text: String },
    /// Type a reply and submit it.
    Reply { agent: String, text: String },
    /// Accept the highlighted choice on a prompt (Enter).
    Approve { agent: String },
    /// Back out of a prompt (Esc).
    Deny { agent: String },
    /// The conversation the phone currently has open. Only this agent's full
    /// history is published, so the snapshot stays small.
    Focus { agent: String },
    /// Start a new agent in a project. `agent` carries the project id and
    /// `text` the provider, since every write endpoint speaks that shape.
    NewAgent { project: String, provider: String },
}

/// This machine's Tailscale address, if it is on a tailnet.
///
/// Binding here rather than `0.0.0.0` is deliberate: the page is reachable
/// from your own devices and from nothing else, with no firewall rule to get
/// wrong.
pub fn tailscale_addr() -> Option<IpAddr> {
    let output = Command::new("tailscale").arg("ip").arg("-4").output().ok()?;
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .next()?
        .trim()
        .parse()
        .ok()
}

/// A token that is easy to keep in a bookmark and hard to guess.
pub fn new_token() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    (0..24)
        .map(|_| {
            const ALPHABET: &[u8] = b"abcdefghijkmnopqrstuvwxyz23456789";
            ALPHABET[rng.gen_range(0..ALPHABET.len())] as char
        })
        .collect()
}

impl Remote {
    /// Start serving, or explain why we cannot.
    pub fn start(
        port: u16,
        token: String,
        shared: Shared,
        commands: mpsc::UnboundedSender<RemoteCommand>,
        _actions: mpsc::UnboundedSender<Action>,
    ) -> Result<Remote> {
        let ip = tailscale_addr()
            .ok_or_else(|| anyhow!("no Tailscale address; is tailscale running?"))?;
        let addr = SocketAddr::new(ip, port);
        let config = RemoteConfig { addr, token };

        serve_on(addr, &config.token, &shared, &commands)
            .map_err(|err| anyhow!("could not bind {addr}: {err}"))?;

        // Also on loopback, so `tailscale serve` — which proxies to
        // 127.0.0.1 — can put HTTPS in front. That is what unlocks
        // dictation, which browsers refuse outside a secure context.
        let loopback = SocketAddr::new(IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), port);
        if let Err(err) = serve_on(loopback, &config.token, &shared, &commands) {
            crate::logger::warn(format!("phone view not on loopback: {err}"));
        }

        Ok(Remote { config })
    }
}

/// Bind one address and answer requests on it until the process ends.
fn serve_on(
    addr: SocketAddr,
    token: &str,
    shared: &Shared,
    commands: &mpsc::UnboundedSender<RemoteCommand>,
) -> Result<()> {
    let server = Server::http(addr).map_err(|err| anyhow!("{err}"))?;
    let (token, shared, commands) = (token.to_string(), shared.clone(), commands.clone());
    std::thread::spawn(move || {
        for mut request in server.incoming_requests() {
            let response = handle(&mut request, &token, &shared, &commands);
            if let Err(err) = request.respond(response) {
                crate::logger::warn(format!("remote response failed: {err}"));
            }
        }
    });
    Ok(())
}

fn json(body: String) -> Response<std::io::Cursor<Vec<u8>>> {
    let header = Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..])
        .expect("static header parses");
    Response::from_string(body).with_header(header)
}

fn html(body: &str) -> Response<std::io::Cursor<Vec<u8>>> {
    let header = Header::from_bytes(&b"Content-Type"[..], &b"text/html; charset=utf-8"[..])
        .expect("static header parses");
    Response::from_string(body.to_string()).with_header(header)
}

fn status(code: u16, message: &str) -> Response<std::io::Cursor<Vec<u8>>> {
    Response::from_string(message.to_string()).with_status_code(code)
}

fn handle(
    request: &mut tiny_http::Request,
    token: &str,
    shared: &Shared,
    commands: &mpsc::UnboundedSender<RemoteCommand>,
) -> Response<std::io::Cursor<Vec<u8>>> {
    let url = request.url().to_string();
    let (path, query) = url.split_once('?').unwrap_or((url.as_str(), ""));

    if !authorized(request, &query_params(query), token) {
        return status(401, "unauthorized");
    }

    match (request.method().as_str(), path) {
        ("GET", "/") => html(page::HTML),
        ("GET", "/api/state") => match shared.lock() {
            Ok(snapshot) => json(serde_json::to_string(&*snapshot).unwrap_or_default()),
            Err(_) => status(500, "state unavailable"),
        },
        ("POST", "/api/todo") => command_from(request, commands, |agent, text| {
            (!text.is_empty()).then_some(RemoteCommand::Todo { agent, text })
        }),
        ("POST", "/api/reply") => command_from(request, commands, |agent, text| {
            (!text.is_empty()).then_some(RemoteCommand::Reply { agent, text })
        }),
        ("POST", "/api/approve") => {
            command_from(request, commands, |agent, _| Some(RemoteCommand::Approve { agent }))
        }
        ("POST", "/api/deny") => {
            command_from(request, commands, |agent, _| Some(RemoteCommand::Deny { agent }))
        }
        ("POST", "/api/focus") => {
            command_from(request, commands, |agent, _| Some(RemoteCommand::Focus { agent }))
        }
        ("POST", "/api/new-agent") => command_from(request, commands, |project, provider| {
            Some(RemoteCommand::NewAgent { project, provider })
        }),
        _ => status(404, "not found"),
    }
}

/// The token may travel in the bookmark's query string or a header. Both are
/// fine over the tailnet; the query form is what makes a home-screen icon work.
fn authorized(request: &tiny_http::Request, params: &[(String, String)], token: &str) -> bool {
    if params
        .iter()
        .any(|(key, value)| key == "t" && value == token)
    {
        return true;
    }
    request.headers().iter().any(|header| {
        header.field.equiv("Authorization") && header.value.as_str() == format!("Bearer {token}")
    })
}

fn query_params(query: &str) -> Vec<(String, String)> {
    query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            (key.to_string(), percent_decode(value))
        })
        .collect()
}

/// Enough decoding for a token and a short id; bodies are JSON, not forms.
fn percent_decode(value: &str) -> String {
    let bytes = value.replace('+', " ").into_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(byte) = u8::from_str_radix(&String::from_utf8_lossy(&bytes[i + 1..i + 3]), 16)
            {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn command_from(
    request: &mut tiny_http::Request,
    commands: &mpsc::UnboundedSender<RemoteCommand>,
    build: impl Fn(String, String) -> Option<RemoteCommand>,
) -> Response<std::io::Cursor<Vec<u8>>> {
    let mut body = String::new();
    if std::io::Read::read_to_string(request.as_reader(), &mut body).is_err() {
        return status(400, "unreadable body");
    }
    let Some((agent, text)) = parse_command_body(&body) else {
        return status(400, "expected {\"agent\": \"…\"}");
    };
    match build(agent, text) {
        Some(command) => {
            if commands.send(command).is_err() {
                return status(503, "workbench is shutting down");
            }
            json("{\"ok\":true}".to_string())
        }
        None => status(400, "nothing to do"),
    }
}

/// `{"agent": "ab12cd34", "text": "…"}` → the pair, or nothing if malformed.
fn parse_command_body(body: &str) -> Option<(String, String)> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    let agent = value.get("agent")?.as_str()?.trim().to_string();
    if agent.is_empty() {
        return None;
    }
    let text = value
        .get("text")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    Some((agent, text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_command_body_needs_an_agent() {
        assert_eq!(
            parse_command_body(r#"{"agent":"ab12cd34","text":"do the thing"}"#),
            Some(("ab12cd34".to_string(), "do the thing".to_string()))
        );
        // Approve/deny carry no text.
        assert_eq!(
            parse_command_body(r#"{"agent":"ab12cd34"}"#),
            Some(("ab12cd34".to_string(), String::new()))
        );
        assert_eq!(parse_command_body(r#"{"text":"orphan"}"#), None);
        assert_eq!(parse_command_body(r#"{"agent":"  "}"#), None);
        assert_eq!(parse_command_body("not json"), None);
    }

    #[test]
    fn the_token_survives_the_trip_through_a_bookmark() {
        let params = query_params("t=abc%2Ddef&x=1");
        assert_eq!(params[0], ("t".to_string(), "abc-def".to_string()));
        assert_eq!(params[1], ("x".to_string(), "1".to_string()));
        assert!(query_params("").is_empty());
    }

    #[test]
    fn tokens_are_long_and_unguessable_enough_to_sit_in_a_url() {
        let a = new_token();
        let b = new_token();
        assert_eq!(a.len(), 24);
        assert_ne!(a, b);
        // No look-alike characters: this gets read off a screen sometimes.
        assert!(!a.contains('l') && !a.contains('1') && !a.contains('0'));
    }
}
