//! LF-only JSONL framing.
//!
//! This is the smallest module in the crate and the one most likely to corrupt a
//! stream, so it is pure, it has no dependencies, and it is tested against the cases
//! that actually break a line reader:
//!
//! * **The separator is `\n` and nothing else.** `\r\n` is a record terminator with a
//!   carriage return glued on, which is stripped; `U+2028` (LINE SEPARATOR) and
//!   `U+2029` (PARAGRAPH SEPARATOR) are *content*, and a model that quotes one of them
//!   must not turn one record into two. A reader that splits on "a Unicode line break"
//!   fails exactly here, and only on the inputs that matter.
//! * **The unit is bytes, and the boundary is not a character boundary.** A chunk from a
//!   socket or a pipe may end in the middle of a multi-byte character; the partial line
//!   stays buffered and the character is decoded once its bytes are all present. A reader
//!   that decoded each chunk on arrival would corrupt the last character of many records.
//! * **A line that is not UTF-8 is reported, not hidden.** [`LineSplitter::push`] returns
//!   `String`s so a caller can keep reading, and records the loss in
//!   [`LineSplitter::had_invalid_utf8`]. A stream that has gone binary is a stream whose
//!   session must be refused, and the caller can only refuse it if it is told.
//!
//! There is no `Result` here on purpose: framing cannot fail, it can only be fed more
//! bytes or hit its bound. [`MAX_BUFFERED_BYTES`] is that bound — a peer that never sends
//! a newline must not grow this buffer until the process dies.

/// The most bytes that may sit in the buffer with no newline to release them.
///
/// Twenty-four megabytes: far more than any record a coding agent writes, small enough
/// that a runaway peer is a reported failure rather than an out-of-memory kill.
pub const MAX_BUFFERED_BYTES: usize = 24 * 1024 * 1024;

/// Split a byte stream into JSONL records.
#[derive(Debug, Default)]
pub struct LineSplitter {
    buffer: Vec<u8>,
    invalid_utf8: bool,
    overflowed: bool,
}

impl LineSplitter {
    /// An empty splitter.
    pub fn new() -> Self {
        LineSplitter {
            buffer: Vec::new(),
            invalid_utf8: false,
            overflowed: false,
        }
    }

    /// Feed a chunk and take every record it completed.
    ///
    /// The returned records are in stream order and exclude their terminator. A chunk
    /// that completes no record returns an empty vector; a chunk that completes three
    /// returns three.
    pub fn push(&mut self, chunk: &[u8]) -> Vec<String> {
        self.buffer.extend_from_slice(chunk);
        if self.buffer.len() > MAX_BUFFERED_BYTES {
            self.overflowed = true;
        }
        let mut lines = Vec::new();
        let mut start = 0usize;
        while let Some(offset) = self.buffer[start..].iter().position(|byte| *byte == b'\n') {
            let end = start + offset;
            let content_end = if end > start && self.buffer[end - 1] == b'\r' {
                end - 1
            } else {
                end
            };
            let (line, lossy) = decode(&self.buffer[start..content_end]);
            if lossy {
                self.invalid_utf8 = true;
            }
            lines.push(line);
            start = end + 1;
        }
        if start > 0 {
            self.buffer.drain(..start);
        }
        lines
    }

    /// The trailing partial record, if there is one, and clear the buffer.
    ///
    /// Called at end of stream. A file whose last line has no terminator is *not* a
    /// truncated file — the record is complete, only the newline is missing — so this
    /// returns it. A caller that expected an unterminated fragment (a torn write) must
    /// compare the result against what it can parse, which is what
    /// [`crate::session::parse_session`] does.
    pub fn finish(&mut self) -> Option<String> {
        if self.buffer.is_empty() {
            return None;
        }
        let bytes = std::mem::take(&mut self.buffer);
        let content_end = if bytes.last() == Some(&b'\r') {
            bytes.len() - 1
        } else {
            bytes.len()
        };
        let (line, lossy) = decode(&bytes[..content_end]);
        if lossy {
            self.invalid_utf8 = true;
        }
        if line.is_empty() {
            None
        } else {
            Some(line)
        }
    }

    /// How many bytes are held back awaiting a newline.
    pub fn buffered_len(&self) -> usize {
        self.buffer.len()
    }

    /// True when nothing is held back.
    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    /// True when some record arrived with bytes that are not UTF-8.
    pub fn had_invalid_utf8(&self) -> bool {
        self.invalid_utf8
    }

    /// True when a peer sent more than [`MAX_BUFFERED_BYTES`] with no newline.
    pub fn is_overflowed(&self) -> bool {
        self.overflowed
    }

    /// A short reason the stream is unusable, for the caller's error.
    pub fn reject_reason(&self) -> Option<&'static str> {
        if self.overflowed {
            Some("the peer sent more than the framing buffer bound with no newline")
        } else if self.invalid_utf8 {
            Some("a record arrived with bytes that are not UTF-8")
        } else {
            None
        }
    }
}

/// Split a complete document with the same rules, for replay and tests.
pub fn iter_lines(text: &str) -> Vec<String> {
    let mut splitter = LineSplitter::new();
    let mut lines = splitter.push(text.as_bytes());
    if let Some(tail) = splitter.finish() {
        lines.push(tail);
    }
    lines
}

/// Decode one record, reporting whether it was lossy.
fn decode(bytes: &[u8]) -> (String, bool) {
    match std::str::from_utf8(bytes) {
        Ok(text) => (text.to_string(), false),
        Err(_) => (String::from_utf8_lossy(bytes).into_owned(), true),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chunk_with_three_records_yields_three() {
        let mut splitter = LineSplitter::new();
        let lines = splitter.push(b"one\ntwo\nthree\n");
        assert_eq!(lines, vec!["one", "two", "three"]);
        assert!(splitter.is_empty());
    }

    #[test]
    fn a_partial_record_stays_buffered() {
        let mut splitter = LineSplitter::new();
        assert!(splitter.push(b"{\"type\":\"ag").is_empty());
        assert_eq!(splitter.buffered_len(), 11);
        let lines = splitter.push(b"ent\"}\n");
        assert_eq!(lines, vec!["{\"type\":\"agent\"}"]);
        assert!(splitter.is_empty());
    }

    #[test]
    fn a_carriage_return_is_stripped_and_nothing_else_is() {
        let mut splitter = LineSplitter::new();
        assert_eq!(splitter.push(b"a\r\nb\r\n"), vec!["a", "b"]);
        // A bare `\r` inside a record is content.
        let mut splitter = LineSplitter::new();
        assert_eq!(splitter.push(b"a\rb\n"), vec!["a\rb"]);
    }

    #[test]
    fn the_unicode_line_and_paragraph_separators_do_not_split() {
        // U+2028 and U+2029 in UTF-8 are E2 80 A8 and E2 80 A9: no 0x0A byte anywhere.
        let payload = "{\"text\":\"a\u{2028}b\u{2029}c\"}";
        let mut splitter = LineSplitter::new();
        let lines = splitter.push(format!("{payload}\n").as_bytes());
        assert_eq!(lines.len(), 1, "the payload is one record: {lines:?}");
        assert_eq!(lines[0], payload);
        assert!(!splitter.had_invalid_utf8());
    }

    #[test]
    fn a_multibyte_character_split_across_chunks_round_trips() {
        // "é" is C3 A9; cut between the two bytes.
        let bytes = "é".as_bytes();
        assert_eq!(bytes.len(), 2);
        let mut splitter = LineSplitter::new();
        assert!(splitter.push(&bytes[..1]).is_empty());
        assert_eq!(splitter.buffered_len(), 1);
        let mut second = bytes[1..].to_vec();
        second.push(b'\n');
        let lines = splitter.push(&second);
        assert_eq!(lines, vec!["é".to_string()]);
        assert!(
            !splitter.had_invalid_utf8(),
            "a split character is not invalid UTF-8"
        );
    }

    #[test]
    fn invalid_utf8_is_reported_rather_than_hidden() {
        let mut splitter = LineSplitter::new();
        let lines = splitter.push(&[0xff, 0xfe, b'\n']);
        assert_eq!(lines.len(), 1);
        assert!(
            splitter.had_invalid_utf8(),
            "the caller must be able to refuse the stream"
        );
        assert!(splitter.reject_reason().unwrap().contains("UTF-8"));
    }

    #[test]
    fn finish_returns_the_unterminated_tail_once() {
        let mut splitter = LineSplitter::new();
        assert!(splitter.push(b"a\nb").len() == 1);
        assert_eq!(splitter.finish(), Some("b".to_string()));
        assert_eq!(splitter.finish(), None);
        assert!(splitter.is_empty());
    }

    #[test]
    fn finish_strips_a_trailing_carriage_return_and_drops_an_empty_tail() {
        let mut splitter = LineSplitter::new();
        assert!(splitter.push(b"a\r").is_empty());
        assert_eq!(splitter.finish(), Some("a".to_string()));

        let mut splitter = LineSplitter::new();
        assert!(splitter.push(b"\r").is_empty());
        assert_eq!(splitter.finish(), None);
    }

    #[test]
    fn iter_lines_matches_the_splitter() {
        assert_eq!(iter_lines("a\nb\n"), vec!["a", "b"]);
        assert_eq!(iter_lines("a\nb"), vec!["a", "b"]);
        assert_eq!(iter_lines(""), Vec::<String>::new());
        assert_eq!(iter_lines("\n"), vec![""]);
        assert_eq!(iter_lines("x\r\n"), vec!["x"]);
    }

    #[test]
    fn an_empty_chunk_yields_nothing() {
        let mut splitter = LineSplitter::new();
        assert!(splitter.push(b"").is_empty());
        assert!(splitter.push(b"").is_empty());
        assert!(splitter.is_empty());
    }

    #[test]
    fn a_runaway_peer_is_reported_rather_than_growing_forever() {
        let mut splitter = LineSplitter::new();
        let chunk = vec![b'x'; 1024 * 1024];
        for _ in 0..(MAX_BUFFERED_BYTES / chunk.len() + 1) {
            splitter.push(&chunk);
        }
        assert!(splitter.is_overflowed());
        assert!(splitter.reject_reason().unwrap().contains("newline"));
    }
}
