use crate::app::REPLAY_PARSER_ROWS;
use crate::app::RawOutputBuffer;

/// Create a replay parser by feeding all stored raw bytes through a fresh vt100 parser.
/// The replay parser has REPLAY_PARSER_ROWS rows (500), giving us deep scrollback.
/// After replaying, its screen contains the last 500 rows of terminal output.
pub fn create_replay_parser(raw_buf: &RawOutputBuffer, cols: u16) -> vt100::Parser {
    let mut parser = vt100::Parser::new(REPLAY_PARSER_ROWS, cols, 0);
    // VecDeque may be split across two slices
    let (front, back): (&[u8], &[u8]) = raw_buf.bytes.as_slices();
    if !front.is_empty() {
        parser.process(front);
    }
    if !back.is_empty() {
        parser.process(back);
    }
    parser
}
