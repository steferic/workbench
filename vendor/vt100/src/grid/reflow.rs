//! Rewrap the normal buffer's soft lines without replaying cursor commands.
use super::{Grid, Pos, Size};
use crate::{cell::Cell, row::Row};

impl Grid {
    pub fn resize_reflow(&mut self, size: Size) {
        if self.rows.is_empty() || self.scroll_region_active() {
            self.set_size(size);
            return;
        }
        let history_len = self.scrollback.len();
        let cursor_row = history_len + usize::from(self.pos.row);
        let saved_row = history_len + usize::from(self.saved_pos.row);
        let old: Vec<_> = self.history_rows().cloned().collect();
        let last_content = old
            .iter()
            .rposition(|r| r.cells().any(|c| c.has_contents()))
            .unwrap_or(0);
        let last = last_content
            .max(cursor_row)
            .min(old.len().saturating_sub(1));
        let mut lines: Vec<Row> = Vec::new();
        let mut group: Vec<Cell> = Vec::new();
        let mut cursor_in_group = None;
        let mut saved_in_group = None;
        let mut new_cursor = (0, 0);
        let mut new_saved = (0, 0);
        for (i, row) in old.iter().enumerate().take(last + 1) {
            let row_cells: Vec<_> = row.cells().cloned().collect();
            if i == cursor_row {
                cursor_in_group = Some(group.len() + usize::from(self.pos.col));
            }
            if i == saved_row {
                saved_in_group =
                    Some(group.len() + usize::from(self.saved_pos.col));
            }
            let keep = if row.wrapped() {
                // Wide characters wrap before a final unused column. That
                // padding cell is not a space in the logical line.
                row_cells.len()
                    - usize::from(
                        row_cells.last().is_some_and(|c| *c == Cell::default()),
                    )
            } else {
                row_cells
                    .iter()
                    .rposition(|c| *c != Cell::default())
                    .map_or(0, |n| n + 1)
                    .max(if i == cursor_row {
                        usize::from(self.pos.col)
                    } else {
                        0
                    })
            };
            group.extend(row_cells.into_iter().take(keep));
            if row.wrapped() && i < last {
                continue;
            }
            let base = lines.len();
            let (mut wrapped, positions) = wrap(&group, size.cols);
            let locate =
                |offset: usize| positions[offset.min(positions.len() - 1)];
            if let Some(offset) = cursor_in_group.take() {
                let (r, c) = locate(offset);
                new_cursor = (base + r, c);
            }
            if let Some(offset) = saved_in_group.take() {
                let (r, c) = locate(offset);
                new_saved = (base + r, c);
            }
            lines.append(&mut wrapped);
            group.clear();
        }
        let split = lines.len().saturating_sub(usize::from(size.rows));
        self.rows = lines.split_off(split);
        self.scrollback = lines.into();
        while self.scrollback.len() > self.scrollback_len {
            self.scrollback.pop_front();
        }
        self.size = size;
        self.rows
            .resize_with(usize::from(size.rows), || Row::new(size.cols));
        self.pos = Pos {
            row: new_cursor
                .0
                .saturating_sub(split)
                .min(usize::from(size.rows - 1)) as u16,
            col: new_cursor.1.min(size.cols),
        };
        self.saved_pos = Pos {
            row: new_saved
                .0
                .saturating_sub(split)
                .min(usize::from(size.rows - 1)) as u16,
            col: new_saved.1.min(size.cols - 1),
        };
        self.scroll_top = 0;
        self.scroll_bottom = size.rows - 1;
        self.scrollback_offset =
            self.scrollback_offset.min(self.scrollback.len());
    }
}

fn wrap(cells: &[Cell], cols: u16) -> (Vec<Row>, Vec<(usize, u16)>) {
    let mut rows = vec![Row::new(cols)];
    let mut positions = vec![(0, 0); cells.len() + 1];
    let mut col = 0;
    let mut i = 0;
    while i < cells.len() {
        let wide = cells[i].is_wide();
        let width = if wide { 2 } else { 1 };
        if col > 0 && col + width > cols {
            rows.last_mut().unwrap().wrap(true);
            rows.push(Row::new(cols));
            col = 0;
        }
        positions[i] = (rows.len() - 1, col);
        // A one-column viewport cannot display a wide glyph, but must not
        // destroy its continuation cell before the window is widened again.
        if wide && cols == 1 {
            rows.last_mut().unwrap().resize(2, Cell::default());
        }
        *rows.last_mut().unwrap().get_mut(col).unwrap() = cells[i].clone();
        if wide && i + 1 < cells.len() && cells[i + 1].is_wide_continuation() {
            positions[i + 1] = (rows.len() - 1, col);
            if let Some(cell) = rows.last_mut().unwrap().get_mut(col + 1) {
                *cell = cells[i + 1].clone();
            }
            i += 1;
        }
        col = (col + width).min(cols);
        i += 1;
        positions[i] = (rows.len() - 1, col);
    }
    (rows, positions)
}
