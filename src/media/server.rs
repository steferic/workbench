use super::{Library, Store};
use crate::remote::http::Request;
use std::{
    io::{Read, Seek, SeekFrom, Write},
    net::TcpListener,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, Weak,
    },
    time::Duration,
};
use tiny_http::{Header, Response, ResponseBox, StatusCode};

#[derive(Debug)]
pub(super) struct Local {
    pub port: u16,
    pub token: String,
    stop: Arc<AtomicBool>,
}
impl Drop for Local {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}
impl Local {
    pub(super) fn start(store: Weak<Mutex<Store>>) -> anyhow::Result<Self> {
        let listener = Arc::new(TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))?);
        listener.set_nonblocking(true)?;
        let port = listener.local_addr()?.port();
        let token = uuid::Uuid::new_v4().to_string();
        let stop = Arc::new(AtomicBool::new(false));
        for _ in 0..4 {
            let (listener, store, token, stop) =
                (listener.clone(), store.clone(), token.clone(), stop.clone());
            std::thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            // Accepted sockets must block even though accept is nonblocking.
                            let _ = stream.set_nonblocking(false);
                            let mut head = false;
                            let response = match Request::read(&mut stream) {
                                Ok(request) => {
                                    head = request.method().as_str() == "HEAD";
                                    match store.upgrade() {
                                        Some(store) => response(
                                            &Library(store),
                                            request.url(),
                                            &request,
                                            &token,
                                        ),
                                        None => break,
                                    }
                                }
                                Err(_) => text(400, "Invalid request"),
                            };
                            let _ = response.raw_print(
                                &mut stream,
                                tiny_http::HTTPVersion(1, 1),
                                &[],
                                head,
                                None,
                            );
                            let _ = stream.flush();
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(25))
                        }
                        Err(_) => break,
                    }
                }
            });
        }
        Ok(Self { port, token, stop })
    }
}

fn header(k: &str, v: &str) -> Header {
    Header::from_bytes(k, v).unwrap()
}
fn text(code: u16, message: &str) -> ResponseBox {
    Response::from_string(message)
        .with_status_code(code)
        .with_header(header("Cache-Control", "no-store"))
        .boxed()
}

pub(super) fn response(
    library: &Library,
    url: &str,
    request: &Request,
    token: &str,
) -> ResponseBox {
    let (path, query) = url.split_once('?').unwrap_or((url, ""));
    let authorized = url::form_urlencoded::parse(query.as_bytes())
        .any(|(k, v)| k == "t" && v == token)
        || request.headers().iter().any(|h| {
            h.field.equiv("Authorization") && h.value.as_str() == format!("Bearer {token}")
        });
    if !authorized {
        return text(401, "Unauthorized");
    }
    if !matches!(request.method().as_str(), "GET" | "HEAD") {
        return text(405, "Use GET or HEAD");
    }
    let Some(tail) = path.strip_prefix("/media/") else {
        return text(404, "Not found");
    };
    let (id, mode) = tail.split_once('/').unwrap_or((tail, ""));
    let Some(artifact) = library.get(id) else {
        return text(404, "This preview has expired or its agent was deleted.");
    };
    if mode.is_empty() {
        let body = VIEWER
            .replace("MEDIA_NAME", &escape(&artifact.view.name))
            .replace(
                "MEDIA_ELEMENT",
                if artifact.view.kind == "video" {
                    "video"
                } else {
                    "img"
                },
            )
            .replace("MEDIA_SRC", &format!("/media/{id}/file?t={token}"));
        return text(200, &body).with_header(header("Content-Type","text/html; charset=utf-8"))
            .with_header(header("Referrer-Policy","no-referrer"))
            .with_header(header("Content-Security-Policy","default-src 'none'; img-src 'self'; media-src 'self'; style-src 'unsafe-inline'; script-src 'unsafe-inline'; base-uri 'none'; frame-ancestors 'none'"));
    }
    let (file, mime) = match mode {
        "file" => (&artifact.file, artifact.mime),
        "poster" => match artifact.poster.as_ref() {
            Some(p) => (p, "image/png"),
            None => return text(404, "No poster"),
        },
        _ => return text(404, "Not found"),
    };
    let Ok(mut file) = file.reopen() else {
        return text(404, "Preview unavailable");
    };
    let Ok(metadata) = file.metadata() else {
        return text(404, "Preview unavailable");
    };
    let len = metadata.len();
    let range = request
        .headers()
        .iter()
        .find(|h| h.field.equiv("Range"))
        .map(|h| h.value.as_str());
    let (start, end, code) = if let Some(range) = range {
        match byte_range(range, len) {
            Some((a, b)) => (a, b, 206),
            None => {
                return text(416, "Range not satisfiable")
                    .with_header(header("Content-Range", &format!("bytes */{len}")))
            }
        }
    } else {
        (0, len.saturating_sub(1), 200)
    };
    if file.seek(SeekFrom::Start(start)).is_err() {
        return text(500, "Could not seek media");
    }
    let size = end + 1 - start;
    let mut headers = vec![
        header("Content-Type", mime),
        header("Accept-Ranges", "bytes"),
        header("Cache-Control", "private, no-store"),
        header("X-Content-Type-Options", "nosniff"),
        header("Connection", "close"),
    ];
    if code == 206 {
        headers.push(header(
            "Content-Range",
            &format!("bytes {start}-{end}/{len}"),
        ));
    }
    let body: Box<dyn Read + Send> = if request.method().as_str() == "HEAD" {
        Box::new(std::io::empty())
    } else {
        Box::new(file.take(size))
    };
    Response::new(StatusCode(code), headers, body, Some(size as usize), None)
}

fn byte_range(raw: &str, len: u64) -> Option<(u64, u64)> {
    if len == 0 {
        return None;
    }
    let (start, end) = raw.strip_prefix("bytes=")?.split_once('-')?;
    if start.is_empty() {
        let count = end.parse::<u64>().ok()?;
        return (count > 0).then_some((len.saturating_sub(count), len - 1));
    }
    let start = start.parse::<u64>().ok()?;
    let end = if end.is_empty() {
        len - 1
    } else {
        end.parse::<u64>().ok()?.min(len - 1)
    };
    (start < len && start <= end).then_some((start, end))
}
fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

const VIEWER: &str = r#"<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>MEDIA_NAME · Workbench</title><style>
:root{color-scheme:light dark;font:16px system-ui}body{margin:0;background:light-dark(#fafafa,#141418);color:light-dark(#18181b,#eee);min-height:100dvh;display:grid;grid-template-rows:auto 1fr auto}header,footer{padding:16px 24px}h1{font-size:16px;margin:0;overflow-wrap:anywhere}main{display:flex;align-items:center;justify-content:center;min-height:0;padding:16px}video,img{max-width:100%;max-height:75dvh;object-fit:contain}a{color:inherit;display:inline-flex;align-items:center;min-height:44px}#error{max-width:40em;line-height:1.5}a:focus-visible{outline:2px solid currentColor;outline-offset:4px}
</style><header><h1>MEDIA_NAME</h1></header><main><MEDIA_ELEMENT id="media" src="MEDIA_SRC" alt="MEDIA_NAME" controls playsinline preload="metadata"></MEDIA_ELEMENT><p id="error" hidden role="status">This browser could not display the file. It may use an unsupported codec, or the preview may have expired. Download it to open with a local player.</p></main><footer><a href="MEDIA_SRC" download="MEDIA_NAME">Download original</a></footer><script>const media=document.getElementById('media');media.addEventListener('error',()=>{media.hidden=true;document.getElementById('error').hidden=false})</script></html>"#;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ranges_support_browser_seeking_and_reject_invalid_requests() {
        for (s, r) in [
            ("bytes=0-9", Some((0, 9))),
            ("bytes=95-", Some((95, 99))),
            ("bytes=-10", Some((90, 99))),
            ("bytes=90-999", Some((90, 99))),
            ("bytes=100-", None),
            ("bytes=9-2", None),
            ("bytes=0-1,5-9", None),
            ("bytes=-0", None),
        ] {
            assert_eq!(byte_range(s, 100), r);
        }
    }
}
