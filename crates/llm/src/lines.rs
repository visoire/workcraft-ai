//! Turn a byte stream into lines, and an SSE stream into events.

/// Collects bytes and hands out complete lines (without the line ending).
#[derive(Debug, Default)]
pub struct LineBuffer {
    buf: Vec<u8>,
}

impl LineBuffer {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a chunk and return every line it completed.
    pub fn push(&mut self, chunk: &[u8]) -> Vec<String> {
        self.buf.extend_from_slice(chunk);
        let mut out = Vec::new();
        while let Some(pos) = self.buf.iter().position(|b| *b == b'\n') {
            let rest = self.buf.split_off(pos + 1);
            let mut line = std::mem::replace(&mut self.buf, rest);
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            out.push(String::from_utf8_lossy(&line).into_owned());
        }
        out
    }

    /// What is left when the stream ends without a final newline.
    pub fn finish(&mut self) -> Option<String> {
        if self.buf.is_empty() {
            return None;
        }
        let s = String::from_utf8_lossy(&self.buf).into_owned();
        self.buf.clear();
        Some(s)
    }
}

/// One server-sent event.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SseEvent {
    pub event: Option<String>,
    pub data: String,
}

/// Builds SSE events from lines: `event:` and `data:` fields, a blank line ends the event.
#[derive(Debug, Default)]
pub struct SseParser {
    event: Option<String>,
    data: Vec<String>,
}

impl SseParser {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push_line(&mut self, line: &str) -> Option<SseEvent> {
        if line.is_empty() {
            return self.flush();
        }
        if line.starts_with(':') {
            return None;
        }
        let (field, value) = match line.split_once(':') {
            Some((f, v)) => (f, v.strip_prefix(' ').unwrap_or(v)),
            None => (line, ""),
        };
        match field {
            "event" => self.event = Some(value.to_string()),
            "data" => self.data.push(value.to_string()),
            _ => {}
        }
        None
    }

    pub fn flush(&mut self) -> Option<SseEvent> {
        if self.data.is_empty() && self.event.is_none() {
            return None;
        }
        let ev = SseEvent {
            event: self.event.take(),
            data: self.data.join("\n"),
        };
        self.data.clear();
        Some(ev)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_across_chunks() {
        let mut b = LineBuffer::new();
        assert!(b.push(b"{\"a\":").is_empty());
        let l = b.push(b"1}\r\n{\"b\":2}\npart");
        assert_eq!(l, vec!["{\"a\":1}", "{\"b\":2}"]);
        assert_eq!(b.finish().as_deref(), Some("part"));
        assert_eq!(b.finish(), None);
    }

    #[test]
    fn multibyte_split_across_chunks() {
        let mut b = LineBuffer::new();
        let bytes = "ä\n".as_bytes();
        assert!(b.push(&bytes[..1]).is_empty());
        assert_eq!(b.push(&bytes[1..]), vec!["ä"]);
    }

    #[test]
    fn sse_events() {
        let mut p = SseParser::new();
        assert_eq!(p.push_line("event: message_start"), None);
        assert_eq!(p.push_line("data: {\"x\":1}"), None);
        let ev = p.push_line("");
        assert_eq!(
            ev,
            Some(SseEvent {
                event: Some("message_start".into()),
                data: "{\"x\":1}".into()
            })
        );
        assert_eq!(p.push_line(": comment"), None);
        assert_eq!(p.push_line("data: [DONE]"), None);
        assert_eq!(p.push_line("").map(|e| e.data), Some("[DONE]".to_string()));
    }
}
