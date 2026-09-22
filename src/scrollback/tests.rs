use super::*;
use crate::app::TranscriptBuffer;
use std::io::Write;
use unicode_width::UnicodeWidthStr;

fn render(text: &str, width: u16) -> Vec<TranscriptLine> {
    render::layout(
        &render::prose(text, &Palette::from_theme(Theme::DARK)),
        width,
    )
}
fn text(lines: &[TranscriptLine]) -> String {
    lines
        .iter()
        .map(TranscriptLine::text)
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn markdown_formats_nested_styles_lists_quotes_and_links() {
    let rows = render("## Heading\n\n- a **bold and *italic*** [linked label](https://example.com/a(b)) followed by more words\n  - nested\n\n> quoted text\n\n`code` and ~~removed~~", 28);
    let plain = text(&rows);
    assert!(!plain.contains("**"));
    assert!(!plain.contains("##"));
    assert!(plain.contains("• nested"));
    assert!(plain.contains("│ quoted text"));
    assert!(rows
        .iter()
        .flat_map(|r| r.spans())
        .any(|s| s.text.contains("italic")
            && s.style
                .add_modifier
                .contains(Modifier::BOLD | Modifier::ITALIC)));
    let links: Vec<_> = rows.iter().flat_map(|r| &r.links).collect();
    assert!(!links.is_empty());
    assert!(links
        .iter()
        .all(|l| l.target == "https://example.com/a(b)" && l.end <= 28));
    assert!(rows.iter().all(|r| r.text().width() <= 28));
}

#[test]
fn code_indentation_and_characters_survive_narrowing_and_widening() {
    let source = "```rust\n    let long_name = \"alpha beta gamma\";\n        nested();\n```";
    let wide = render(source, 80);
    let narrow = render(source, 12);
    assert_eq!(
        text(&wide),
        "    let long_name = \"alpha beta gamma\";\n        nested();"
    );
    assert_eq!(
        narrow.iter().map(TranscriptLine::text).collect::<String>(),
        wide.iter().map(TranscriptLine::text).collect::<String>()
    );
    assert_eq!(render(source, 80), wide);
    assert!(narrow.iter().all(|r| r.text().width() <= 12));
}

#[test]
fn tables_wrap_inside_columns_and_keep_all_cells_in_narrow_panes() {
    let source = "| Name | Description |\n| --- | --- |\n| Alpha | longer description with several words |\n| Beta | 界界 Unicode |";
    for width in [5, 18, 24, 60] {
        let rows = render(source, width);
        assert!(
            rows.iter().all(|r| r.text().width() <= width as usize),
            "{width}: {}",
            text(&rows)
        );
        let flat: String = text(&rows)
            .chars()
            .filter(|c| !c.is_whitespace() && *c != '│')
            .collect();
        assert!(
            flat.contains("Alpha") && flat.contains("Beta") && flat.contains("Unicode"),
            "{flat}"
        );
        if width >= 24 {
            let positions: Vec<_> = rows
                .iter()
                .filter_map(|r| r.text().find('│').map(|i| r.text()[..i].width()))
                .collect();
            assert!(positions.windows(2).all(|w| w[0] == w[1]));
        }
    }
}

#[test]
fn unicode_and_bare_links_keep_destinations_when_wrapped() {
    let url = "https://example.com/a/very/long/path";
    let rows = render(&format!("界界 e\u{301} {url}"), 8);
    assert!(rows.iter().all(|r| r.text().width() <= 8));
    assert!(rows.iter().flat_map(|r| &r.links).all(|l| l.target == url));
    assert!(rows.iter().flat_map(|r| &r.links).count() > 1);
    for width in [1, 2, 7, 15] {
        assert!(render("long ASCII words", width)
            .iter()
            .all(|r| r.text().width() <= width as usize));
    }
    let rows = render("👩‍💻👩‍💻 e\u{301}", 2);
    assert!(rows.iter().all(|r| r.text().width() <= 2));
    assert_eq!(
        rows.iter().map(TranscriptLine::text).collect::<String>(),
        "👩‍💻👩‍💻 e\u{301}"
    );
}

#[test]
fn selection_uses_terminal_columns_and_the_last_history_row_is_reachable() {
    let mut history = TranscriptBuffer::new(100);
    history.set_log_history(Some(render("界 e\u{301} 👩‍💻 last\n\nfinal paragraph", 40)));
    assert_eq!(history.extract_text((0, 3), (0, 3)), "e\u{301}");
    assert_eq!(history.extract_text((0, 5), (0, 6)), "👩‍💻");
    let (scroll, row) = history.reading_position(1, 2);
    assert_eq!(scroll, 1);
    assert_eq!(row, history.len().saturating_sub(2 + scroll));
    assert_eq!(history.line(row + 1), Some("final paragraph"));
}

#[test]
fn both_provider_logs_keep_complete_tool_results_and_skip_other_roles() {
    for format in [LogFormat::Claude, LogFormat::Codex] {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        let records = match format {
            LogFormat::Claude => vec![
                serde_json::json!({"type":"user","message":{"content":"question"}}),
                serde_json::json!({"type":"assistant","message":{"content":[{"type":"text","text":"**answer**"}]}}),
                serde_json::json!({"type":"user","message":{"content":[{"type":"tool_result","content":"first\n    second\nlast"}]}}),
                serde_json::json!({"type":"user","isSidechain":true,"message":{"content":"hidden"}}),
            ],
            LogFormat::Codex => vec![
                serde_json::json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"text":"question"}]}}),
                serde_json::json!({"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"text":"**answer**"}]}}),
                serde_json::json!({"type":"response_item","payload":{"type":"function_call_output","output":"first\n    second\nlast"}}),
                serde_json::json!({"type":"response_item","payload":{"type":"message","role":"system","content":[{"text":"hidden"}]}}),
            ],
        };
        for record in records {
            writeln!(file, "{record}").unwrap();
        }
        let rows = history(format, file.path(), 80, Theme::DARK).unwrap().lines;
        let plain = text(&rows);
        assert!(plain.contains("› question") && plain.contains("answer"));
        assert!(
            plain.contains("first") && plain.contains("        second") && plain.contains("last")
        );
        assert!(!plain.contains("hidden") && !plain.contains("**"));
    }
}

#[test]
fn reading_anchor_survives_new_messages_reflow_and_explicit_scrolling() {
    let initial = (0..15)
        .map(|n| format!("Paragraph {n} contains several words that wrap across rows.\n\n"))
        .collect::<String>();
    let mut buffer = TranscriptBuffer::new(500);
    buffer.set_log_history(Some(render(&initial, 40)));
    let (scroll, row) = buffer.reading_position(15, 5);
    let anchor = buffer.styled_line(row).unwrap().anchor.unwrap();
    let appended = format!("{initial}\nNew answer arriving while reading.");
    buffer.set_log_history(Some(render(&appended, 20)));
    let (scroll, row) = buffer.reading_position(scroll, 5);
    let relocated = buffer.styled_line(row).unwrap().anchor.unwrap();
    assert_eq!(relocated.0, anchor.0);
    assert!(relocated.1 <= anchor.1);
    let (_, moved) = buffer.reading_position(scroll + 2, 5);
    assert_eq!(moved, row.saturating_sub(2));
}

#[test]
fn terminal_history_keeps_text_styles_links_and_cursor_through_resize() {
    let mut parser = vt100::Parser::new(4, 20, 100);
    parser.process(b"\x1b[31m0123456789ABCDEFGHIJ\x1b[0m");
    parser.resize_reflow(4, 10);
    assert_eq!(parser.screen().cursor_position(), (1, 10));
    parser.resize_reflow(4, 20);
    assert_eq!(parser.screen().contents(), "0123456789ABCDEFGHIJ");
    assert_eq!(parser.screen().cursor_position(), (0, 20));
    parser.process(b"\r\n\x1b]8;;https://example.com\x1b\\linked\x1b]8;;\x1b\\\r\n");
    for i in 0..15 {
        parser.process(format!("line {i}\r\n").as_bytes());
    }
    parser.resize_reflow(3, 8);
    let rows = terminal_rows(parser.screen());
    assert!(text(&rows).contains("line 0"));
    assert!(rows
        .iter()
        .flat_map(|r| &r.links)
        .any(|l| l.target == "https://example.com"));
    assert!(rows
        .iter()
        .flat_map(|r| r.spans())
        .any(|s| s.style.fg == Some(ratatui::style::Color::Indexed(1))));
    parser.resize_reflow(4, 30);
    assert!(text(&terminal_rows(parser.screen())).contains("0123456789ABCDEFGHIJ"));
}

#[test]
fn terminal_reflow_preserves_wide_and_combining_characters() {
    let mut parser = vt100::Parser::new(4, 12, 100);
    parser.process("界界e\u{301}界 abc".as_bytes());
    let original = parser.screen().contents();
    for width in [4, 1, 7, 12] {
        parser.resize_reflow(4, width);
    }
    assert_eq!(parser.screen().contents(), original);
    let mut parser = vt100::Parser::new(4, 4, 100);
    parser.process("a界b".as_bytes());
    parser.resize_reflow(4, 2);
    assert_eq!(
        terminal_rows(parser.screen())
            .iter()
            .map(TranscriptLine::text)
            .collect::<String>(),
        "a界b"
    );
    parser.resize_reflow(4, 4);
    assert_eq!(parser.screen().contents(), "a界b");
}
