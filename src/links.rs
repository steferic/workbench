//! Link destinations are metadata, never escape sequences embedded in text.
use ratatui::layout::Rect;
use std::sync::OnceLock;
use unicode_width::UnicodeWidthStr;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    pub row: usize,
    pub start: usize,
    pub end: usize,
    pub target: String,
}

#[derive(Clone, Debug)]
pub struct Hit {
    pub area: Rect,
    pub target: String,
}

/// Only open links through the OS, with no shell interpretation or executable schemes.
pub fn destination(raw: &str) -> Option<String> {
    if raw.len() > 8192 || raw.chars().any(char::is_control) {
        return None;
    }
    let url = url::Url::parse(raw).ok()?;
    match url.scheme() {
        "http" | "https" if url.host_str().is_some() => Some(url.into()),
        "mailto" => Some(url.into()),
        "file" if url.host_str().is_none_or(|h| h == "localhost") => {
            url.to_file_path().ok()?;
            Some(url.into())
        }
        _ => None,
    }
}

/// Returns an activation only for a press/release on the same, still-visible link.
pub fn mouse(
    pressed: &mut Option<(u16, u16, String)>,
    hits: &[Hit],
    action: &crate::app::Action,
) -> Option<String> {
    use crate::app::Action;
    match action {
        Action::MouseClick(x, y) => {
            *pressed = hits
                .iter()
                .find(|hit| hit.area.contains((*x, *y).into()))
                .map(|hit| (*x, *y, hit.target.clone()));
        }
        Action::MouseDrag(..) | Action::Resize(..) => {
            *pressed = None;
        }
        Action::MouseUp(x, y) => {
            let (px, py, target) = pressed.take()?;
            if (px, py) == (*x, *y)
                && hits
                    .iter()
                    .any(|hit| hit.area.contains((*x, *y).into()) && hit.target == target)
            {
                return Some(target);
            }
        }
        _ => {}
    }
    None
}

pub fn open(raw: &str) -> anyhow::Result<()> {
    let target = destination(raw).ok_or_else(|| anyhow::anyhow!("Unsupported link destination"))?;
    // Reap the launcher off the UI thread. No detached media player is owned here.
    std::thread::spawn(move || {
        let mut command = std::process::Command::new(if cfg!(target_os = "macos") {
            "open"
        } else {
            "xdg-open"
        });
        let result = command
            .arg(target)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        if !result.is_ok_and(|status| status.success()) {
            crate::logger::warn("Could not open link in the system browser");
        }
    });
    Ok(())
}

pub fn plain(text: &str, row: usize) -> Vec<Link> {
    static URLS: OnceLock<regex::Regex> = OnceLock::new();
    let re = URLS.get_or_init(|| {
        regex::Regex::new(r#"(?:https?://|file://|mailto:)[^\s<>\"\x00-\x1f]+"#).unwrap()
    });
    re.find_iter(text)
        .filter_map(|m| {
            let mut raw = m
                .as_str()
                .trim_end_matches(['.', ',', ';', '!', '?', '\'', '`', ']']);
            while raw.ends_with(')') && raw.matches(')').count() > raw.matches('(').count() {
                raw = &raw[..raw.len() - 1];
            }
            let target = destination(raw)?;
            let start = text[..m.start()].width();
            Some(Link {
                row,
                start,
                end: start + raw.width(),
                target,
            })
        })
        .collect()
}

pub fn screen(screen: &vt100::Screen, start: usize, count: usize) -> Vec<Link> {
    let mut links: Vec<Link> = Vec::new();
    let mut destinations = std::collections::HashMap::new();
    for row in start..(start + count).min(screen.size().0 as usize) {
        let mut text = String::new();
        for col in 0..screen.size().1 {
            let Some(cell) = screen.cell(row as u16, col) else {
                continue;
            };
            if !cell.is_wide_continuation() {
                let contents = cell.contents();
                text.push_str(if contents.is_empty() { " " } else { &contents });
            }
            let Some(raw) = cell.hyperlink() else {
                continue;
            };
            let Some(target) = destinations
                .entry(raw)
                .or_insert_with(|| destination(raw))
                .clone()
            else {
                continue;
            };
            let col = col as usize;
            if let Some(previous) = links
                .last_mut()
                .filter(|l| l.row == row - start && l.end == col && l.target == target)
            {
                previous.end += 1;
            } else {
                links.push(Link {
                    row: row - start,
                    start: col,
                    end: col + 1,
                    target,
                });
            }
        }
        for link in plain(&text, row - start) {
            if !links
                .iter()
                .any(|l| l.row == link.row && l.start < link.end && link.start < l.end)
            {
                links.push(link);
            }
        }
    }
    links
}

pub fn hits(links: &[Link], area: Rect) -> Vec<Hit> {
    links
        .iter()
        .filter_map(|link| {
            let end = link.end.min(area.width as usize);
            if link.row >= area.height as usize || link.start >= end {
                return None;
            }
            Some(Hit {
                area: Rect::new(
                    area.x + link.start as u16,
                    area.y + link.row as u16,
                    (end - link.start) as u16,
                    1,
                ),
                target: link.target.clone(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn osc_links_survive_split_reads_sgr_wide_cells_and_scrolling() {
        let bytes = "\x1b]8;id=one;https://example.com/a;b\x1b\\\x1b[4m界abc\x1b[0mD\x1b]8;;\x07 plain\r\nnext\r\nlast".as_bytes();
        for split in 0..bytes.len() {
            let mut p = vt100::Parser::new(2, 30, 10);
            p.process(&bytes[..split]);
            p.process(&bytes[split..]);
            p.set_scrollback(1);
            let links = screen(p.screen(), 0, 2);
            assert_eq!(
                links[0],
                Link {
                    row: 0,
                    start: 0,
                    end: 6,
                    target: "https://example.com/a;b".into()
                }
            );
        }
    }
    #[test]
    fn overwrite_erase_and_reset_remove_links() {
        for erase in ["\rxxxxx", "\r\x1b[2K", "\x1bc"] {
            let mut p = vt100::Parser::new(3, 30, 0);
            p.process(b"\x1b]8;;https://example.com\x07hello\x1b]8;;\x07");
            p.process(erase.as_bytes());
            assert!(screen(p.screen(), 0, 3).is_empty());
        }
    }
    #[test]
    fn links_are_clipped_and_plain_urls_use_display_columns() {
        let links = plain("界 https://example.com/a(b).", 0);
        assert_eq!(links[0].start, 3);
        assert_eq!(links[0].target, "https://example.com/a(b)");
        assert_eq!(
            hits(&links, Rect::new(10, 5, 8, 2))[0].area,
            Rect::new(13, 5, 5, 1)
        );
        for bad in [
            "javascript:alert(1)",
            "data:text/html,test",
            "file://evil/etc/passwd",
            "https://example.com/\x1b",
        ] {
            assert!(destination(bad).is_none());
        }
    }
}

#[cfg(test)]
mod interaction_tests {
    use super::*;
    use crate::app::Action;
    #[test]
    fn clicks_open_links_but_dragging_or_changed_content_does_not() {
        let hits = vec![Hit {
            area: Rect::new(10, 10, 20, 1),
            target: "https://example.com/".into(),
        }];
        let mut press = None;
        assert!(mouse(&mut press, &hits, &Action::MouseClick(12, 10)).is_none());
        assert_eq!(
            mouse(&mut press, &hits, &Action::MouseUp(12, 10)),
            Some(hits[0].target.clone())
        );
        mouse(&mut press, &hits, &Action::MouseClick(12, 10));
        mouse(&mut press, &hits, &Action::MouseDrag(18, 10));
        assert!(mouse(&mut press, &hits, &Action::MouseUp(12, 10)).is_none());
        mouse(&mut press, &hits, &Action::MouseClick(12, 10));
        assert!(mouse(&mut press, &[], &Action::MouseUp(12, 10)).is_none());
        mouse(&mut press, &hits, &Action::MouseClick(12, 10));
        mouse(&mut press, &hits, &Action::Resize(80, 24));
        assert!(mouse(&mut press, &hits, &Action::MouseUp(12, 10)).is_none());
    }
}
