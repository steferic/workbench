//! Agent-owned, bounded media snapshots. Only explicitly presented files are served.
#[cfg(test)]
mod fixture;
mod preview;
mod server;
use anyhow::{bail, Context, Result};
pub use preview::{
    browse, close, flush_cleanup, latest, present_if_visible, render, resized, tick, Preview,
};
use serde::Serialize;
use std::{
    collections::VecDeque,
    fs::File,
    io::{Read, Write},
    path::Path,
    sync::{Arc, Mutex},
};
use uuid::Uuid;

const MAX_FILE: u64 = 256 * 1024 * 1024;
const MAX_LIBRARY: u64 = 512 * 1024 * 1024;

#[derive(Debug, Clone, Serialize)]
pub struct View {
    pub id: String,
    pub agent: String,
    pub name: String,
    pub kind: String,
    pub poster: bool,
}

#[derive(Debug)]
pub struct Artifact {
    pub view: View,
    pub mime: &'static str,
    pub file: tempfile::NamedTempFile,
    pub poster: Option<tempfile::NamedTempFile>,
    size: u64,
}

#[derive(Clone, Default, Debug)]
pub struct Library(Arc<Mutex<Store>>);
#[derive(Default, Debug)]
struct Store {
    items: VecDeque<Arc<Artifact>>,
    local: Option<server::Local>,
    closed: bool,
    importing: Arc<Mutex<()>>,
    import_control: Arc<ImportControl>,
}

impl Library {
    pub fn shutdown(&self) {
        if let Ok(mut store) = self.0.lock() {
            store.closed = true;
            store.import_control.cancel();
            store.local.take();
            store.items.clear();
        }
    }
    pub fn get(&self, id: &str) -> Option<Arc<Artifact>> {
        self.0
            .lock()
            .ok()?
            .items
            .iter()
            .find(|a| a.view.id == id)
            .cloned()
    }
    pub fn remove(&self, id: &str) {
        if let Ok(mut store) = self.0.lock() {
            store.items.retain(|a| a.view.id != id);
        }
    }
    pub fn views(&self) -> Vec<View> {
        self.0
            .lock()
            .map(|s| s.items.iter().map(|a| a.view.clone()).collect())
            .unwrap_or_default()
    }
    pub fn retain_agents(&self, agents: &[String]) {
        if let Ok(mut store) = self.0.lock() {
            store.items.retain(|a| agents.contains(&a.view.agent));
        }
    }
    pub fn present(&self, agent: String, path: &Path) -> Result<Arc<Artifact>> {
        let (gate, control) = {
            let store = self
                .0
                .lock()
                .map_err(|_| anyhow::anyhow!("Media library unavailable"))?;
            if store.closed {
                bail!("Workbench is closing");
            }
            (store.importing.clone(), store.import_control.clone())
        };
        let _import = gate
            .lock()
            .map_err(|_| anyhow::anyhow!("Media importer unavailable"))?;
        let mut source = File::open(path).context("Could not open media file")?;
        let metadata = source.metadata()?;
        if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_FILE {
            bail!("Present a regular, nonempty file no larger than 256 MiB");
        }
        let mut head = [0; 32];
        let n = source.read(&mut head)?;
        let image_format = image::guess_format(&head[..n]).ok();
        let (kind, mime) = match image_format {
            Some(image::ImageFormat::Png) => ("image", "image/png"),
            Some(image::ImageFormat::Jpeg) => ("image", "image/jpeg"),
            Some(image::ImageFormat::Gif) => ("image", "image/gif"),
            Some(image::ImageFormat::WebP) => ("image", "image/webp"),
            _ if n >= 12 && &head[4..8] == b"ftyp" => ("video", "video/mp4"),
            _ if n >= 4 && head[..4] == [0x1a, 0x45, 0xdf, 0xa3] => ("video", "video/webm"),
            _ => bail!("Supported previews: PNG, JPEG, GIF, WebP, MP4/MOV and WebM"),
        };
        if kind == "image" && metadata.len() > 25 * 1024 * 1024 {
            bail!("Images must be no larger than 25 MiB");
        }
        let mut file = tempfile::Builder::new()
            .prefix("workbench-media-")
            .tempfile()?;
        file.write_all(&head[..n])?;
        let copied = std::io::copy(
            &mut std::io::Read::by_ref(&mut source).take(MAX_FILE + 1 - n as u64),
            &mut file,
        )? + n as u64;
        let after = source.metadata()?;
        if copied != metadata.len()
            || copied > MAX_FILE
            || after.len() != metadata.len()
            || after.modified().ok() != metadata.modified().ok()
        {
            bail!(
                "Media file changed while being copied; present it again after the render finishes"
            );
        }
        file.flush()?;
        if kind == "image" {
            let (w, h) = image::ImageReader::open(file.path())?
                .with_guessed_format()?
                .into_dimensions()?;
            if w == 0 || h == 0 || u64::from(w) * u64::from(h) > 32_000_000 {
                bail!("Image exceeds the 32 megapixel preview limit");
            }
        }
        let poster = if kind == "video" {
            make_poster(file.path(), &control)
        } else {
            None
        };
        let artifact = Arc::new(Artifact {
            view: View {
                id: Uuid::new_v4().to_string(),
                agent,
                name: path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .chars()
                    .filter(|c| !c.is_control())
                    .take(200)
                    .collect(),
                kind: kind.into(),
                poster: poster.is_some(),
            },
            mime,
            size: copied,
            file,
            poster,
        });
        let mut store = self
            .0
            .lock()
            .map_err(|_| anyhow::anyhow!("Media library unavailable"))?;
        if store.closed {
            bail!("Workbench is closing");
        }
        while store
            .items
            .iter()
            .filter(|a| a.view.agent == artifact.view.agent)
            .count()
            >= 16
        {
            if let Some(i) = store
                .items
                .iter()
                .position(|a| a.view.agent == artifact.view.agent)
            {
                store.items.remove(i);
            }
        }
        while store.items.len() >= 64
            || store.items.iter().map(|a| a.size).sum::<u64>() + copied > MAX_LIBRARY
        {
            store.items.pop_front();
        }
        store.items.push_back(artifact.clone());
        Ok(artifact)
    }
    pub fn local_url(&self, id: &str) -> Result<String> {
        let mut store = self
            .0
            .lock()
            .map_err(|_| anyhow::anyhow!("Media library unavailable"))?;
        if store.closed {
            bail!("Workbench is closing");
        }
        if store.local.is_none() {
            store.local = Some(server::Local::start(Arc::downgrade(&self.0))?);
        }
        let local = store.local.as_ref().unwrap();
        Ok(format!(
            "http://127.0.0.1:{}/media/{}?t={}",
            local.port, id, local.token
        ))
    }
    pub(crate) fn response(
        &self,
        path: &str,
        request: &crate::remote::http::Request,
        token: &str,
    ) -> tiny_http::Response<Box<dyn Read + Send>> {
        server::response(self, path, request, token)
    }
}

#[derive(Default, Debug)]
struct ImportControl {
    canceled: std::sync::atomic::AtomicBool,
    child: Mutex<Option<std::process::Child>>,
}
impl ImportControl {
    fn stop_child(&self) {
        if let Ok(mut slot) = self.child.lock() {
            if let Some(mut child) = slot.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
    fn cancel(&self) {
        self.canceled
            .store(true, std::sync::atomic::Ordering::Relaxed);
        self.stop_child();
    }
}
fn make_poster(path: &Path, control: &ImportControl) -> Option<tempfile::NamedTempFile> {
    let file = tempfile::Builder::new()
        .prefix("workbench-poster-")
        .suffix(".png")
        .tempfile()
        .ok()?;
    let mut slot = control.child.lock().ok()?;
    if control.canceled.load(std::sync::atomic::Ordering::Relaxed) {
        return None;
    }
    *slot = Some(
        std::process::Command::new("ffmpeg")
            .args([
                "-nostdin",
                "-loglevel",
                "error",
                "-y",
                "-protocol_whitelist",
                "file,pipe",
                "-i",
            ])
            .arg(path)
            .args([
                "-frames:v",
                "1",
                "-vf",
                "scale=960:540:force_original_aspect_ratio=decrease",
                "-threads",
                "1",
            ])
            .arg(file.path())
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .ok()?,
    );
    drop(slot);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let status = {
            let mut slot = control.child.lock().ok()?;
            let status = slot.as_mut()?.try_wait();
            if matches!(status, Ok(Some(_))) {
                slot.take();
            }
            status
        };
        match status {
            Ok(Some(status)) => {
                return (status.success() && file.as_file().metadata().ok()?.len() > 0)
                    .then_some(file)
            }
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(25))
            }
            _ => {
                control.stop_child();
                return None;
            }
        }
    }
}

pub fn cli(path: std::path::PathBuf, agent: Option<String>) -> Result<()> {
    let path = path.canonicalize().context("Media file not found")?;
    let agent = agent
        .or_else(|| std::env::var(crate::comms::ENV_SESSION).ok())
        .ok_or_else(|| {
            anyhow::anyhow!("Use --agent <id> when presenting outside an agent session")
        })?;
    let mut client = crate::control::Client::connect()?;
    client.set_timeout(Some(std::time::Duration::from_secs(30)))?;
    let agents = client.call("agents.list", serde_json::json!({}))?;
    let agents = agents.as_array().cloned().unwrap_or_default();
    // Presenting to our own session is expected, including by alias or id prefix.
    let mut scope = crate::control::Scope::from_env();
    scope.exclude = None;
    let agent = crate::control::resolve_agent(&agents, &agent, &scope)?;
    let result = client.call(
        "media.present",
        serde_json::json!({"agent":agent,"path":path}),
    )?;
    println!(
        "Preview ready: {}\n{}",
        result["name"].as_str().unwrap_or("media"),
        result["url"].as_str().unwrap_or("")
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[cfg(unix)]
    fn shutdown_reaps_an_active_thumbnail_process_and_closes_imports() {
        let library = Library::default();
        let child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .unwrap();
        let pid = child.id();
        let control = library.0.lock().unwrap().import_control.clone();
        *control.child.lock().unwrap() = Some(child);
        library.shutdown();
        assert!(control.child.lock().unwrap().is_none());
        assert_eq!(unsafe { libc::kill(pid as i32, 0) }, -1);
        assert!(library.present("agent".into(), png().path()).is_err());
    }
    fn png() -> tempfile::NamedTempFile {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        image::DynamicImage::new_rgb8(80, 40)
            .write_to(&mut file, image::ImageFormat::Png)
            .unwrap();
        file
    }
    #[test]
    fn presented_files_are_snapshots_and_are_removed_with_their_owner() {
        let source = png();
        let library = Library::default();
        let item = library.present("agent".into(), source.path()).unwrap();
        let original = std::fs::read(item.file.path()).unwrap();
        std::fs::write(source.path(), b"changed source").unwrap();
        assert_eq!(std::fs::read(item.file.path()).unwrap(), original);
        let copy = item.file.path().to_owned();
        let id = item.view.id.clone();
        drop(item);
        library.retain_agents(&[]);
        assert!(!copy.exists());
        assert!(library.get(&id).is_none());
    }
    #[test]
    fn library_evicts_old_previews_and_rejects_non_media() {
        let library = Library::default();
        let source = png();
        let first = library.present("agent".into(), source.path()).unwrap();
        let first_id = first.view.id.clone();
        let first_path = first.file.path().to_owned();
        drop(first);
        for _ in 0..16 {
            library.present("agent".into(), source.path()).unwrap();
        }
        assert_eq!(library.views().len(), 16);
        assert!(library.get(&first_id).is_none());
        assert!(!first_path.exists());
        std::fs::write(source.path(), b"<script>alert(1)</script>").unwrap();
        assert!(library.present("agent".into(), source.path()).is_err());
    }
    fn request(url: &str, extra: &str) -> Vec<u8> {
        request_method("GET", url, extra)
    }
    fn request_method(method: &str, url: &str, extra: &str) -> Vec<u8> {
        let url = url::Url::parse(url).unwrap();
        let mut stream = std::net::TcpStream::connect(("127.0.0.1", url.port().unwrap())).unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        write!(
            stream,
            "{method} {}?{} HTTP/1.1\r\nHost: localhost\r\n{}\r\n",
            url.path(),
            url.query().unwrap_or(""),
            extra
        )
        .unwrap();
        let mut data = Vec::new();
        stream.read_to_end(&mut data).unwrap();
        data
    }
    #[test]
    fn browser_routes_require_auth_stream_ranges_and_expire() {
        let source = png();
        let library = Library::default();
        let item = library.present("agent".into(), source.path()).unwrap();
        let page = library.local_url(&item.view.id).unwrap();
        assert!(String::from_utf8_lossy(&request(&page, "")).starts_with("HTTP/1.1 200"));
        assert!(
            String::from_utf8_lossy(&request(page.split('?').next().unwrap(), ""))
                .starts_with("HTTP/1.1 401")
        );
        let file_url = page.replace("?t=", "/file?t=");
        for url in [&page, &file_url] {
            let head = request_method("HEAD", url, "");
            assert!(String::from_utf8_lossy(&head).starts_with("HTTP/1.1 200"));
            assert!(head.ends_with(b"\r\n\r\n"), "HEAD must not send a body");
            let get = request(url, "");
            assert!(get.len() > head.len());
        }
        let bytes = request(&file_url, "Range: bytes=0-7\r\n");
        assert!(String::from_utf8_lossy(&bytes).starts_with("HTTP/1.1 206"));
        assert!(bytes.ends_with(b"\x89PNG\r\n\x1a\n"));
        assert!(
            String::from_utf8_lossy(&request(&file_url, "Range: bytes=999999-\r\n"))
                .starts_with("HTTP/1.1 416")
        );
        library.retain_agents(&[]);
        assert!(String::from_utf8_lossy(&request(&page, "")).starts_with("HTTP/1.1 404"));
        library.shutdown();
    }
}
