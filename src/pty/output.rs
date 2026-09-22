use vte::Perform;

/// The reader's terminal state has the real PTY dimensions and no scrollback.
/// It answers queries immediately, independently of the UI's output queue.
pub(super) struct TerminalOutput {
    screen: vt100::Parser,
    queries: vte::Parser,
    detector: Detector,
    reflow: bool,
}

pub(super) struct Output {
    pub bytes: Vec<u8>,
    pub replies: Vec<u8>,
}

impl TerminalOutput {
    pub fn new(rows: u16, cols: u16, strip_alt_screen: bool) -> Self {
        Self {
            reflow: false,
            screen: vt100::Parser::new(rows.max(1), cols.max(1), 0),
            queries: vte::Parser::new(),
            detector: Detector {
                event: None,
                strip_alt_screen,
            },
        }
    }

    pub fn screen(&self) -> &vt100::Screen {
        self.screen.screen()
    }

    pub fn with_reflow(mut self, reflow: bool, scrollback_rows: usize) -> Self {
        if reflow {
            let (rows, cols) = self.screen.screen().size();
            self.screen = vt100::Parser::new(rows, cols, scrollback_rows);
        }
        self.reflow = reflow;
        self
    }

    pub fn resize(&mut self, rows: u16, cols: u16) {
        let size = (rows.max(1), cols.max(1));
        if self.screen.screen().size() != size {
            if self.reflow {
                self.screen.resize_reflow(size.0, size.1);
            } else {
                self.screen.set_size(size.0, size.1);
            }
        }
    }

    pub fn process(&mut self, data: &[u8]) -> Output {
        let mut output = Output {
            bytes: Vec::with_capacity(data.len()),
            replies: Vec::new(),
        };
        let mut start = 0;
        let mut parsed = 0;
        for (i, &byte) in data.iter().enumerate() {
            self.queries.advance(&mut self.detector, byte);
            let Some(event) = self.detector.event.take() else {
                continue;
            };
            output.bytes.extend_from_slice(&data[start..i]);
            match &event {
                Event::Modes(retained) => {
                    // A CSI prefix may already have reached the UI in an earlier
                    // read. Cancel it, then re-emit any modes we are keeping.
                    // This also handles mixed sequences like ?1049;25h without
                    // losing the cursor-visibility setting.
                    output.bytes.push(0x18);
                    output.bytes.extend_from_slice(retained);
                }
                Event::Query(_) => output.bytes.push(byte),
            }
            self.screen.process(&output.bytes[parsed..]);
            parsed = output.bytes.len();
            start = i + 1;
            if let Event::Query(query) = event {
                match query {
                    Query::Cursor => {
                        let screen = self.screen.screen();
                        let (rows, cols) = screen.size();
                        let (row, col) = screen.cursor_position();
                        // vt100 represents pending autowrap with col == cols.
                        // CPR still reports the last physical cell in that case.
                        output.replies.extend_from_slice(
                            format!("\x1b[{};{}R", row.min(rows - 1) + 1, col.min(cols - 1) + 1,)
                                .as_bytes(),
                        );
                    }
                    Query::Status => output.replies.extend_from_slice(b"\x1b[0n"),
                    Query::Primary => output.replies.extend_from_slice(b"\x1b[?6c"),
                    Query::Secondary => output.replies.extend_from_slice(b"\x1b[>0;0;0c"),
                }
            }
        }
        output.bytes.extend_from_slice(&data[start..]);
        self.screen.process(&output.bytes[parsed..]);
        output
    }
}

enum Query {
    Cursor,
    Status,
    Primary,
    Secondary,
}
enum Event {
    Query(Query),
    Modes(Vec<u8>),
}

struct Detector {
    event: Option<Event>,
    strip_alt_screen: bool,
}

impl Perform for Detector {
    fn csi_dispatch(
        &mut self,
        params: &vte::Params,
        intermediates: &[u8],
        ignore: bool,
        action: char,
    ) {
        if ignore {
            return;
        }
        let mut values = params.iter();
        let first = values.next();
        if values.next().is_none() {
            let query = match (intermediates, action, first) {
                ([], 'n', Some([6])) => Some(Query::Cursor),
                ([], 'n', Some([5])) => Some(Query::Status),
                ([], 'c', Some([0])) => Some(Query::Primary),
                ([b'>'], 'c', Some([0])) => Some(Query::Secondary),
                _ => None,
            };
            if let Some(query) = query {
                self.event = Some(Event::Query(query));
                return;
            }
        }
        if self.strip_alt_screen && intermediates == b"?" && matches!(action, 'h' | 'l') {
            let is_alt = |p: &&[u16]| matches!(*p, [47] | [1047] | [1049]);
            if params.iter().any(|p| is_alt(&p)) {
                let retained: Vec<String> = params
                    .iter()
                    .filter(|p| !is_alt(p))
                    .map(|p| p.iter().map(u16::to_string).collect::<Vec<_>>().join(":"))
                    .collect();
                let replacement = if retained.is_empty() {
                    Vec::new()
                } else {
                    format!("\x1b[?{}{action}", retained.join(";")).into_bytes()
                };
                self.event = Some(Event::Modes(replacement));
            }
        }
    }

    fn esc_dispatch(&mut self, intermediates: &[u8], ignore: bool, byte: u8) {
        if !ignore && intermediates.is_empty() && byte == b'Z' {
            self.event = Some(Event::Query(Query::Primary));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reflow_keeps_reader_replies_aligned_with_the_ui_parser() {
        let mut reader = TerminalOutput::new(4, 20, false).with_reflow(true, 100);
        let mut ui = vt100::Parser::new(4, 20, 100);
        let bytes = "abcdefghij界klmnopqrstuvwxyz".repeat(8);
        ui.process(&reader.process(bytes.as_bytes()).bytes);
        for (rows, cols) in [(3, 8), (5, 30), (4, 12)] {
            reader.resize(rows, cols);
            ui.resize_reflow(rows, cols);
            assert_eq!(
                reader.screen().cursor_position(),
                ui.screen().cursor_position()
            );
            let (row, col) = ui.screen().cursor_position();
            let reply = reader.process(b"\x1b[6n").replies;
            assert_eq!(
                reply,
                format!("\x1b[{};{}R", row + 1, col.min(cols - 1) + 1).as_bytes()
            );
        }
    }

    #[test]
    fn replies_describe_the_cursor_at_each_query_not_the_end_of_the_read() {
        let mut terminal = TerminalOutput::new(24, 10, false);
        let out = terminal.process(b"\x1b[6nabcdefghijkl\x1b[6n\r\nend\x1b[6n");
        assert_eq!(out.replies, b"\x1b[1;1R\x1b[2;3R\x1b[3;4R");
    }

    #[test]
    fn queries_and_unicode_work_at_every_possible_read_boundary() {
        let bytes = "界e\u{301}\x1b[6n\x1b[0c\x1b[>c\x1b[5n\x1bZ".as_bytes();
        for split in 0..=bytes.len() {
            let mut terminal = TerminalOutput::new(24, 10, false);
            let mut out = terminal.process(&bytes[..split]);
            let next = terminal.process(&bytes[split..]);
            out.replies.extend(next.replies);
            out.bytes.extend(next.bytes);
            assert_eq!(
                out.replies, b"\x1b[1;4R\x1b[?6c\x1b[>0;0;0c\x1b[0n\x1b[?6c",
                "split {split}"
            );
            assert_eq!(out.bytes, bytes);
        }
    }

    #[test]
    fn wrapping_save_restore_and_resize_follow_the_terminal_grid() {
        let mut terminal = TerminalOutput::new(5, 10, false);
        assert_eq!(
            terminal.process(b"0123456789\x1b[6n").replies,
            b"\x1b[1;10R"
        );
        assert_eq!(
            terminal.process(b"x\x1b7\x1b[5;10H\x1b8\x1b[6n").replies,
            b"\x1b[2;2R"
        );
        terminal.process(b"\x1b[5;10H");
        terminal.resize(3, 4);
        assert_eq!(
            terminal.process(b"\x1b[6n\x1b[Habcde\x1b[6n").replies,
            b"\x1b[3;4R\x1b[2;2R"
        );
    }

    #[test]
    fn alt_screen_filter_survives_split_reads_and_keeps_other_modes() {
        let bytes = b"\x1b[2;3H\x1b[?1049h\x1b[6n\x1b[?1049;25l\x1b[6n";
        for split in 0..=bytes.len() {
            let mut terminal = TerminalOutput::new(5, 10, true);
            let first = terminal.process(&bytes[..split]);
            let second = terminal.process(&bytes[split..]);
            let mut ui = vt100::Parser::new(5, 10, 0);
            ui.process(&first.bytes);
            ui.process(&second.bytes);
            assert!(!ui.screen().alternate_screen());
            assert!(ui.screen().hide_cursor());
            assert_eq!(ui.screen().cursor_position(), (1, 2));
            assert_eq!(
                [first.replies, second.replies].concat(),
                b"\x1b[2;3R\x1b[2;3R"
            );
        }
        let mut terminal = TerminalOutput::new(5, 10, false);
        terminal.process(b"\x1b[?1049h");
        assert!(terminal.screen.screen().alternate_screen());
    }

    #[test]
    fn ordinary_output_and_osc_payloads_do_not_trigger_queries() {
        let bytes =
            b"hello\x1b]0;title [6n and [0c\x07\x1b[31mred\x1b[0m\x1b[?2026hframe\x1b[?2026l";
        let mut terminal = TerminalOutput::new(24, 80, true);
        let out = terminal.process(bytes);
        assert_eq!(out.bytes, bytes);
        assert!(out.replies.is_empty());
    }
}
