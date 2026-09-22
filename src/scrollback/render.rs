//! CommonMark blocks shared by every structured-log provider. Layout happens
//! after parsing, so a resize never changes the underlying conversation.
use super::{span, Line, Palette};
use crate::app::TranscriptLine;
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use ratatui::style::{Modifier, Style};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

#[derive(Clone, Debug)]
pub(super) enum Block {
    Flow(Line, usize),
    Literal(Line),
    Table(Vec<Vec<Line>>),
}

pub(super) fn prose(text: &str, p: &Palette) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut line = Vec::new();
    let mut styles = vec![p.assistant];
    let mut links = vec![None];
    let mut lists: Vec<Option<u64>> = Vec::new();
    let mut quote = 0;
    let mut indent = 0;
    let mut code = false;
    let mut code_text = String::new();
    let mut table: Option<Vec<Vec<Line>>> = None;
    let mut row = Vec::new();
    let flush = |blocks: &mut Vec<Block>, line: &mut Line, indent| {
        if !line.is_empty() {
            blocks.push(Block::Flow(std::mem::take(line), indent));
        }
    };
    for event in Parser::new_ext(
        text,
        Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS,
    ) {
        match event {
            Event::Start(Tag::Paragraph) => {
                if line.is_empty() {
                    let prefix = format!("{}{}", "│ ".repeat(quote), " ".repeat(indent));
                    if !prefix.is_empty() {
                        line.push(span(prefix, p.marker));
                    }
                }
            }
            Event::End(TagEnd::Paragraph) => {
                flush(&mut blocks, &mut line, indent + quote * 2);
                if lists.is_empty() {
                    blocks.push(Block::Flow(Vec::new(), 0));
                }
            }
            Event::Start(Tag::Heading { .. }) => {
                flush(&mut blocks, &mut line, indent);
                styles.push(p.heading);
            }
            Event::End(TagEnd::Heading(_)) => {
                flush(&mut blocks, &mut line, 0);
                styles.pop();
            }
            Event::Start(Tag::List(start)) => {
                flush(&mut blocks, &mut line, indent);
                lists.push(start);
            }
            Event::End(TagEnd::List(_)) => {
                lists.pop();
                indent = lists.len() * 2;
            }
            Event::Start(Tag::Item) => {
                flush(&mut blocks, &mut line, indent);
                let marker = match lists.last_mut() {
                    Some(Some(n)) => {
                        let marker = format!("{n}. ");
                        *n += 1;
                        marker
                    }
                    _ => "• ".into(),
                };
                let prefix = format!(
                    "{}{}{marker}",
                    "│ ".repeat(quote),
                    "  ".repeat(lists.len().saturating_sub(1))
                );
                indent = prefix.width();
                line.push(span(prefix, p.marker));
            }
            Event::End(TagEnd::Item) => flush(&mut blocks, &mut line, indent),
            Event::Start(Tag::BlockQuote(_)) => {
                flush(&mut blocks, &mut line, indent);
                quote += 1;
            }
            Event::End(TagEnd::BlockQuote(_)) => {
                flush(&mut blocks, &mut line, indent);
                quote = quote.saturating_sub(1);
            }
            Event::Start(Tag::CodeBlock(_)) => {
                flush(&mut blocks, &mut line, indent);
                code = true;
                code_text.clear();
            }
            Event::End(TagEnd::CodeBlock) => {
                for literal in code_text.lines() {
                    blocks.push(Block::Literal(vec![span(
                        literal.replace('\t', "    "),
                        p.code,
                    )]));
                }
                code = false;
            }
            Event::Start(Tag::Table(_)) => {
                flush(&mut blocks, &mut line, indent);
                table = Some(Vec::new());
            }
            Event::End(TagEnd::TableCell) => row.push(std::mem::take(&mut line)),
            Event::End(TagEnd::TableHead | TagEnd::TableRow) => {
                if let Some(table) = &mut table {
                    table.push(std::mem::take(&mut row));
                }
            }
            Event::End(TagEnd::Table) => {
                if let Some(table) = table.take() {
                    blocks.push(Block::Table(table));
                }
            }
            Event::Start(Tag::Strong | Tag::Emphasis | Tag::Strikethrough) => {
                let modifier = match event {
                    Event::Start(Tag::Strong) => Modifier::BOLD,
                    Event::Start(Tag::Emphasis) => Modifier::ITALIC,
                    _ => Modifier::CROSSED_OUT,
                };
                styles.push(
                    styles
                        .last()
                        .copied()
                        .unwrap_or(p.assistant)
                        .add_modifier(modifier),
                );
            }
            Event::End(TagEnd::Strong | TagEnd::Emphasis | TagEnd::Strikethrough) => {
                styles.pop();
            }
            Event::Start(Tag::Link { dest_url, .. } | Tag::Image { dest_url, .. }) => {
                links.push(crate::links::destination(&dest_url));
                styles.push(
                    styles
                        .last()
                        .copied()
                        .unwrap_or(p.assistant)
                        .add_modifier(Modifier::UNDERLINED),
                );
            }
            Event::End(TagEnd::Link | TagEnd::Image) => {
                links.pop();
                styles.pop();
            }
            Event::Text(text) if code => code_text.push_str(&text),
            Event::Text(text) | Event::Html(text) | Event::InlineHtml(text) => {
                let mut part = span(text.to_string(), *styles.last().unwrap_or(&p.assistant));
                part.link = links.last().cloned().flatten();
                line.push(part);
            }
            Event::Code(text) => {
                let mut part = span(text.to_string(), p.code);
                part.link = links.last().cloned().flatten();
                line.push(part);
            }
            Event::SoftBreak => line.push(span(" ", *styles.last().unwrap_or(&p.assistant))),
            Event::HardBreak => flush(&mut blocks, &mut line, indent),
            Event::TaskListMarker(done) => {
                line.push(span(if done { "☑ " } else { "☐ " }, p.marker))
            }
            Event::Rule => {
                flush(&mut blocks, &mut line, indent);
                blocks.push(Block::Literal(vec![span("────────", p.marker)]));
            }
            _ => {}
        }
    }
    flush(&mut blocks, &mut line, indent);
    while matches!(blocks.last(), Some(Block::Flow(line, _)) if line.is_empty()) {
        blocks.pop();
    }
    blocks
}

/// Each physical row names the logical block and character position it came
/// from. These positions survive wrapping and appended log records.
pub(super) fn layout(blocks: &[Block], cols: u16) -> Vec<TranscriptLine> {
    let width = usize::from(cols.max(1));
    let mut result = Vec::new();
    for (block_id, block) in blocks.iter().enumerate() {
        let rows = match block {
            Block::Flow(line, indent) => wrap(line, width, *indent, true),
            Block::Literal(line) => wrap(line, width, 0, false),
            Block::Table(table) => table_rows(table, width),
        };
        for (offset, spans) in rows {
            let mut line = TranscriptLine::from_styled_spans(spans);
            line.anchor = Some((block_id, offset));
            result.push(line);
        }
    }
    result
}

fn cells(line: &Line) -> usize {
    line.iter().map(|s| s.text.width()).sum()
}

pub(super) fn wrap(line: &Line, width: usize, indent: usize, words: bool) -> Vec<(usize, Line)> {
    let width = width.max(1);
    let text: String = line.iter().map(|s| s.text.as_str()).collect();
    let plain_links = crate::links::plain(&text, 0);
    let mut column = 0;
    let mut chars = Vec::new();
    for s in line {
        let clean: String = s.text.chars().filter(|c| !c.is_control()).collect();
        for c in clean.graphemes(true) {
            let link = s.link.clone().or_else(|| {
                plain_links
                    .iter()
                    .find(|l| column >= l.start && column < l.end)
                    .map(|l| l.target.clone())
            });
            chars.push((c.to_owned(), s.style, link));
            column += c.width();
        }
    }
    if chars.is_empty() {
        return vec![(0, Vec::new())];
    }
    let mut start = 0;
    let mut out = Vec::new();
    while start < chars.len() {
        let padding = if start > 0 {
            indent.min(width.saturating_sub(2))
        } else {
            0
        };
        let mut end = start;
        let mut used = padding;
        while end < chars.len() {
            let next = chars[end].0.width();
            if used + next > width && end > start {
                break;
            }
            used += next;
            end += 1;
        }
        if words && end < chars.len() {
            if let Some(space) = (start..end).rev().find(|&i| chars[i].0 == " " && i > start) {
                end = space + 1;
            }
        }
        let mut row = Vec::new();
        if padding > 0 {
            row.push(span(" ".repeat(padding), Style::default()));
        }
        for (c, style, link) in &chars[start..end] {
            match row.last_mut() {
                Some(last) if last.style == *style && last.link == *link => last.text.push_str(c),
                _ => {
                    let mut s = span(c.to_string(), *style);
                    s.link = link.clone();
                    row.push(s);
                }
            }
        }
        out.push((start, row));
        start = end;
    }
    out
}

fn table_rows(table: &[Vec<Line>], width: usize) -> Vec<(usize, Line)> {
    let count = table.iter().map(Vec::len).max().unwrap_or(0);
    if count == 0 {
        return Vec::new();
    }
    let mut result = Vec::new();
    // Keep columns together where they fit. At very narrow widths, label
    // each cell instead of clipping the right-hand columns out of existence.
    if width < count * 5 + (count - 1) * 3 {
        for (r, row) in table.iter().enumerate().skip(1) {
            for (c, cell) in row.iter().enumerate() {
                let mut line = table[0].get(c).cloned().unwrap_or_default();
                line.push(span(": ", Style::default()));
                line.extend(cell.clone());
                for (offset, line) in wrap(&line, width, 0, true) {
                    result.push((r * 1_000_000 + c * 10_000 + offset, line));
                }
            }
        }
        return result;
    }
    let available = width.saturating_sub((count - 1) * 3);
    let mut widths = vec![available / count; count];
    for w in widths.iter_mut().take(available % count) {
        *w += 1;
    }
    for (r, row) in table.iter().enumerate() {
        let wrapped: Vec<_> = (0..count)
            .map(|c| wrap(row.get(c).unwrap_or(&Vec::new()), widths[c], 0, true))
            .collect();
        let height = wrapped.iter().map(Vec::len).max().unwrap_or(1);
        for y in 0..height {
            let mut line = Vec::new();
            for c in 0..count {
                if c > 0 {
                    line.push(span(" │ ", Style::default()));
                }
                let mut part = wrapped[c]
                    .get(y)
                    .map(|(_, l)| l.clone())
                    .unwrap_or_default();
                if r == 0 {
                    for s in &mut part {
                        s.style = s.style.add_modifier(Modifier::BOLD);
                    }
                }
                let padding = widths[c].saturating_sub(cells(&part));
                line.extend(part);
                if c + 1 < count {
                    line.push(span(" ".repeat(padding), Style::default()));
                }
            }
            result.push((r * 1_000_000 + y, line));
        }
    }
    result
}
