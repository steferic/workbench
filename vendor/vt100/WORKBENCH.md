Based on vt100 0.15.2 (MIT). Local patch retains OSC 8 destinations on cells.
The unmodified parser discards OSC 8, making underlined agent links unclickable.

Changes: Cell hyperlink accessor and reference-counted destination; Screen current
OSC 8 state; attach destinations to both halves of wide glyphs; clear on overwrite
and erase. Existing grid operations carry cell metadata through scroll and resize.
SGR resets do not close links; OSC 8 closes and RIS reset do. Workbench reads cells
and keeps destinations in its transcript records. vt100 formatted ANSI serializers
remain text/style-only; Workbench's replay uses original PTY bytes.

Regression coverage is in src/links.rs. Keep this patch until upstream exposes
cell hyperlinks, then remove the crates.io override.
