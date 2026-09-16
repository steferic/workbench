use super::{Artifact, Library};
use crate::app::{AppState, FocusPanel, InputMode};
use ratatui::{
    layout::{Rect, Size},
    style::Style,
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};
use ratatui_image::{
    picker::{Picker, ProtocolType},
    protocol::{kitty::Kitty, Protocol},
    Image, Resize,
};
use std::{
    io::Write,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, OnceLock,
    },
};

pub struct Preview {
    pub artifact: Arc<Artifact>,
    picker: Picker,
    ready: Option<Encoded>,
    pending: Option<mpsc::Receiver<Result<Encoded, String>>>,
    retry: bool,
    requested: Size,
    error: Option<String>,
    garbage: Vec<u32>,
    canceled: Arc<AtomicBool>,
}
struct Encoded {
    protocol: Protocol,
    kitty_id: Option<u32>,
    size: Size,
}
impl Preview {
    fn new(artifact: Arc<Artifact>, picker: Picker) -> Self {
        Self {
            artifact,
            picker,
            ready: None,
            pending: None,
            retry: false,
            requested: Size::new(0, 0),
            error: None,
            garbage: Vec::new(),
            canceled: Arc::new(AtomicBool::new(false)),
        }
    }
    pub fn loading(&self) -> bool {
        self.pending.is_some() || self.retry
    }
    fn prepare(&mut self, size: Size) {
        self.prepare_with(size, worker());
    }
    fn prepare_with(&mut self, size: Size, worker: &mpsc::SyncSender<Job>) {
        self.retry = false;
        if let Some(rx) = self.pending.as_ref() {
            match rx.try_recv() {
                Ok(Ok(encoded)) => {
                    self.pending = None;
                    if let Some(previous) = self.ready.replace(encoded) {
                        self.garbage.extend(previous.kitty_id);
                    }
                }
                Ok(Err(error)) => {
                    self.pending = None;
                    self.error = Some(error);
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.pending = None;
                    self.error = Some("Preview worker stopped".into());
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if self.pending.is_some() || self.requested == size || size.width == 0 || size.height == 0 {
            return;
        }
        self.error = None;
        let artifact = self.artifact.clone();
        if artifact.view.kind == "video" && artifact.poster.is_none() {
            self.requested = size;
            return;
        }
        let picker = self.picker.clone();
        let (tx, rx) = mpsc::sync_channel(1);
        match worker.try_send(Job {
            artifact,
            picker,
            size,
            reply: tx,
            canceled: self.canceled.clone(),
        }) {
            Ok(()) => {
                self.requested = size;
                self.pending = Some(rx);
            }
            // A canceled preview can still occupy the bounded queue. Keep
            // repainting until the worker takes it and this preview can start.
            Err(mpsc::TrySendError::Full(_)) => self.retry = true,
            Err(mpsc::TrySendError::Disconnected(_)) => {
                self.requested = size;
                self.error = Some("Preview worker stopped".into());
            }
        }
    }
    fn retire(mut self) -> Vec<u32> {
        if let Some(ready) = self.ready.take() {
            self.garbage.extend(ready.kitty_id);
        }
        std::mem::take(&mut self.garbage)
    }
}

impl Drop for Preview {
    fn drop(&mut self) {
        self.canceled.store(true, Ordering::Relaxed);
    }
}
struct Job {
    artifact: Arc<Artifact>,
    picker: Picker,
    size: Size,
    reply: mpsc::SyncSender<Result<Encoded, String>>,
    canceled: Arc<AtomicBool>,
}
fn worker() -> &'static mpsc::SyncSender<Job> {
    static WORKER: OnceLock<mpsc::SyncSender<Job>> = OnceLock::new();
    WORKER.get_or_init(|| {
        let (tx, rx) = mpsc::sync_channel::<Job>(1);
        std::thread::spawn(move || {
            while let Ok(job) = rx.recv() {
                if job.canceled.load(Ordering::Relaxed) {
                    continue;
                }
                let result =
                    encode(&job.artifact, &job.picker, job.size).map_err(|e| e.to_string());
                let _ = job.reply.send(result);
            }
        });
        tx
    })
}

fn encode(artifact: &Artifact, picker: &Picker, size: Size) -> anyhow::Result<Encoded> {
    let path = artifact.poster.as_ref().unwrap_or(&artifact.file).path();
    let mut reader = image::ImageReader::open(path)?.with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    let image = reader.decode()?;
    if picker.protocol_type() == ProtocolType::Kitty {
        let font = picker.font_size();
        let image = image.resize(
            u32::from(size.width) * u32::from(font.width),
            u32::from(size.height) * u32::from(font.height),
            image::imageops::FilterType::Triangle,
        );
        let cells = Size::new(
            image.width().div_ceil(u32::from(font.width)) as u16,
            image.height().div_ceil(u32::from(font.height)) as u16,
        );
        let id = (uuid::Uuid::new_v4().as_u128() as u32).max(1);
        let protocol = Protocol::Kitty(Kitty::new(
            image,
            cells,
            id,
            std::env::var_os("TMUX").is_some(),
        )?);
        Ok(Encoded {
            protocol,
            kitty_id: Some(id),
            size,
        })
    } else {
        Ok(Encoded {
            protocol: picker.new_protocol(image, size, Resize::Fit(None))?,
            kitty_id: None,
            size,
        })
    }
}

pub fn library(state: &AppState) -> Library {
    state
        .system
        .remote_state
        .lock()
        .map(|s| s.media_library.clone())
        .unwrap_or_default()
}
pub fn close(state: &mut AppState) {
    if let Some(preview) = state.ui.media_preview.take() {
        state.system.media_delete_ids.extend(preview.retire());
    }
}
pub fn show(state: &mut AppState, id: &str) {
    let Some(artifact) = library(state).get(id) else {
        return;
    };
    close(state);
    state.ui.pressed_link = None;
    state.ui.media_preview = Some(Preview::new(artifact, state.system.media_picker.clone()));
}
fn focused_owner(state: &AppState) -> Option<String> {
    if matches!(state.ui.focus, FocusPanel::PinnedTerminalPane(_)) {
        state
            .pinned_terminal_id_at(state.focused_pinned_pane())
            .and_then(|id| state.get_session(id))
            .map(|s| s.short_id())
    } else {
        state.active_session().map(|s| s.short_id())
    }
}
pub fn latest(state: &mut AppState) {
    let owner = focused_owner(state);
    let media = library(state).views();
    if let Some(item) = media
        .iter()
        .rev()
        .find(|item| Some(&item.agent) == owner.as_ref())
    {
        show(state, &item.id);
    } else {
        state
            .ui
            .set_task_status("This agent has not presented media yet");
    }
}
pub fn browse(state: &mut AppState) {
    let Some(preview) = state.ui.media_preview.as_ref() else {
        return;
    };
    match library(state)
        .local_url(&preview.artifact.view.id)
        .and_then(|url| crate::links::open(&url))
    {
        Ok(()) => {}
        Err(error) => {
            if let Some(preview) = state.ui.media_preview.as_mut() {
                preview.error = Some(error.to_string());
            }
        }
    }
}
pub fn resized(state: &mut AppState) {
    if let Ok(window) = crossterm::terminal::window_size() {
        if window.columns > 0 && window.rows > 0 {
            let font = ratatui_image::FontSize::new(
                window.width / window.columns,
                window.height / window.rows,
            );
            let previous = state.system.media_picker.font_size();
            if font.width > 0
                && font.height > 0
                && (font.width, font.height) != (previous.width, previous.height)
            {
                #[allow(deprecated)]
                let mut picker = Picker::from_fontsize(font);
                picker.set_protocol_type(state.system.media_picker.protocol_type());
                state.system.media_picker = picker;
                if let Some(id) = state
                    .ui
                    .media_preview
                    .as_ref()
                    .map(|p| p.artifact.view.id.clone())
                {
                    show(state, &id);
                }
            }
        }
    }
}
pub fn render(frame: &mut Frame, state: &mut AppState) {
    let Some(preview) = state.ui.media_preview.as_mut() else {
        return;
    };
    let full = frame.area();
    if full.width < 8 || full.height < 6 {
        return;
    }
    let area = Rect::new(full.x + 2, full.y + 1, full.width - 4, full.height - 2);
    let t = crate::theme::current();
    frame.render_widget(Clear, area);
    let title = format!(
        " {} · {} ",
        preview.artifact.view.name, preview.artifact.view.agent
    );
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .style(Style::default().bg(t.bg).fg(t.fg));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let image_area = Rect::new(
        inner.x,
        inner.y,
        inner.width,
        inner.height.saturating_sub(2),
    );
    preview.prepare(image_area.as_size());
    state.system.media_delete_ids.append(&mut preview.garbage);
    if let Some(ready) = preview
        .ready
        .as_ref()
        .filter(|r| r.size == image_area.as_size())
    {
        let size = ready.protocol.size();
        let rect = Rect::new(
            image_area.x + image_area.width.saturating_sub(size.width) / 2,
            image_area.y + image_area.height.saturating_sub(size.height) / 2,
            size.width.min(image_area.width),
            size.height.min(image_area.height),
        );
        frame.render_widget(Image::new(&ready.protocol), rect);
    } else {
        let text = preview.error.as_deref().unwrap_or(if preview.loading() {
            "Preparing preview…"
        } else {
            "Video ready · press Enter to play in your browser"
        });
        frame.render_widget(
            Paragraph::new(text).alignment(ratatui::layout::Alignment::Center),
            image_area,
        );
    }
    let hint = if preview.artifact.view.kind == "video" {
        "Enter play in browser · Esc close"
    } else {
        "B open original in browser · Esc close"
    };
    frame.render_widget(
        Paragraph::new(hint).alignment(ratatui::layout::Alignment::Center),
        Rect::new(inner.x, inner.bottom().saturating_sub(1), inner.width, 1),
    );
}

/// Delete only IDs this overlay owns, on the UI thread between complete frames.
/// Unicode placements disappear with their cells; this also frees stored pixels.
pub fn flush_cleanup(state: &mut AppState) {
    let mut out = std::io::stdout().lock();
    for id in state.system.media_delete_ids.drain(..) {
        let sequence = format!("\x1b_Ga=d,d=I,i={id},q=2\x1b\\");
        if std::env::var_os("TMUX").is_some() {
            let _ = write!(
                out,
                "\x1bPtmux;{}\x1b\\",
                sequence.replace('\x1b', "\x1b\x1b")
            );
        } else {
            let _ = out.write_all(sequence.as_bytes());
        }
    }
    let _ = out.flush();
}

pub fn tick(state: &mut AppState) {
    let library = library(state);
    let owners: Vec<_> = state
        .data
        .sessions
        .values()
        .flatten()
        .map(|s| s.short_id())
        .collect();
    library.retain_agents(&owners);
    if state
        .ui
        .media_preview
        .as_ref()
        .is_some_and(|p| library.get(&p.artifact.view.id).is_none())
    {
        close(state);
    }
}
pub fn present_if_visible(state: &mut AppState, id: &str) {
    let Some(artifact) = library(state).get(id) else {
        return;
    };
    if state.ui.input_mode == InputMode::Normal
        && state.ui.detail.is_none()
        && !state.ui.pending_quit
        && state.ui.pending_delete.is_none()
        && state.ui.servers.dialog.is_none()
        && focused_owner(state).as_deref() == Some(artifact.view.agent.as_str())
    {
        show(state, id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{backend::TestBackend, Terminal};
    fn artifact() -> Arc<Artifact> {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        image::DynamicImage::new_rgb8(160, 90)
            .write_to(&mut file, image::ImageFormat::Png)
            .unwrap();
        Library::default()
            .present("agent".into(), file.path())
            .unwrap()
    }
    #[test]
    fn image_encoding_fits_small_large_and_wide_panes() {
        let artifact = artifact();
        for kind in [ProtocolType::Halfblocks, ProtocolType::Kitty] {
            let mut picker = Picker::halfblocks();
            picker.set_protocol_type(kind);
            for size in [Size::new(20, 8), Size::new(100, 30), Size::new(70, 5)] {
                let encoded = encode(&artifact, &picker, size).unwrap();
                assert!(
                    encoded.protocol.size().width <= size.width
                        && encoded.protocol.size().height <= size.height
                );
                let mut terminal =
                    Terminal::new(TestBackend::new(size.width, size.height)).unwrap();
                terminal
                    .draw(|f| f.render_widget(Image::new(&encoded.protocol), f.area()))
                    .unwrap();
                let text: String = terminal
                    .backend()
                    .buffer()
                    .content
                    .iter()
                    .map(|c| c.symbol())
                    .collect();
                if let Some(id) = encoded.kitty_id {
                    assert!(text.contains(&format!("i={id}")));
                } else {
                    assert!(!text.contains('\x1b'));
                }
            }
        }
    }
    #[test]
    fn replacing_previews_keeps_repainting_until_the_busy_worker_can_accept_them() {
        let (tx, rx) = mpsc::sync_channel(1);
        let artifact = artifact();
        let size = Size::new(20, 8);
        let mut previous = Preview::new(artifact.clone(), Picker::halfblocks());
        previous.prepare_with(size, &tx);
        drop(previous);

        let mut current = Preview::new(artifact, Picker::halfblocks());
        current.prepare_with(size, &tx);
        assert!(
            current.loading(),
            "keep drawing while a canceled job fills the queue"
        );
        let canceled = rx.try_recv().unwrap();
        assert!(canceled.canceled.load(Ordering::Relaxed));
        drop(canceled);

        current.prepare_with(size, &tx);
        let job = rx.try_recv().unwrap();
        assert!(job
            .reply
            .send(encode(&job.artifact, &job.picker, job.size).map_err(|e| e.to_string()))
            .is_ok());
        current.prepare_with(size, &tx);
        assert!(current.ready.is_some());
        assert!(
            !current.loading(),
            "stop repainting once the image is ready"
        );
    }
    #[test]
    fn closing_and_replacing_preview_retire_only_its_image_ids() {
        let mut picker = Picker::halfblocks();
        picker.set_protocol_type(ProtocolType::Kitty);
        let artifact = artifact();
        let mut p = Preview::new(artifact.clone(), picker.clone());
        let encoded = encode(&artifact, &picker, Size::new(20, 8)).unwrap();
        let id = encoded.kitty_id.unwrap();
        p.ready = Some(encoded);
        let canceled = p.canceled.clone();
        assert_eq!(p.retire(), vec![id]);
        assert!(canceled.load(Ordering::Relaxed));
    }
}
