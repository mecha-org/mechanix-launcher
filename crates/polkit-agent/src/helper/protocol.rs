//! The helper's line protocol: what it says, and what it can be sent.
//!
//! The helper writes one line per message, `g_strescape`d. It reads an
//! answer as one line of at most [`MAX_ANSWER`] bytes.

/// The longest answer the helper reads whole (512 bytes with newline and NUL).
pub(super) const MAX_ANSWER: usize = 510;

/// Said when an answer can't be sent.
pub(crate) const UNSENDABLE: &str = "That answer is too long, or has a line break in it";

/// Whether the helper can take `answer` as one line.
pub(crate) fn sendable(answer: &[u8]) -> bool {
    answer.len() <= MAX_ANSWER && !answer.contains(&b'\n') && !answer.contains(&0)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Line {
    Prompt { text: String, echo: bool },
    Info(String),
    Error(String),
    Success,
    Failure,
}

pub(super) fn parse_line(raw: &[u8]) -> Option<Line> {
    let line = unescape(raw);
    let text = |rest: &[u8]| String::from_utf8_lossy(rest).into_owned();
    if let Some(rest) = line.strip_prefix(b"PAM_PROMPT_ECHO_OFF ") {
        return Some(Line::Prompt {
            text: text(rest),
            echo: false,
        });
    }
    if let Some(rest) = line.strip_prefix(b"PAM_PROMPT_ECHO_ON ") {
        return Some(Line::Prompt {
            text: text(rest),
            echo: true,
        });
    }
    if let Some(rest) = line.strip_prefix(b"PAM_ERROR_MSG ") {
        return Some(Line::Error(text(rest)));
    }
    if let Some(rest) = line.strip_prefix(b"PAM_TEXT_INFO ") {
        return Some(Line::Info(text(rest)));
    }
    match line.as_slice() {
        b"SUCCESS" => Some(Line::Success),
        b"FAILURE" => Some(Line::Failure),
        _ => None,
    }
}

/// Undo `g_strescape`, as `g_strcompress` does.
pub(super) fn unescape(s: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        let b = s[i];
        i += 1;
        if b != b'\\' {
            out.push(b);
            continue;
        }
        let Some(&c) = s.get(i) else {
            break;
        };
        i += 1;
        let byte = match c {
            b'0'..=b'7' => {
                let mut value = u32::from(c - b'0');
                for _ in 0..2 {
                    match s.get(i) {
                        Some(&d @ b'0'..=b'7') => {
                            value = value * 8 + u32::from(d - b'0');
                            i += 1;
                        }
                        _ => break,
                    }
                }
                // As g_strcompress: past `\377`, the low byte.
                (value & 0xff) as u8
            }
            b'b' => 0x08,
            b'f' => 0x0c,
            b'n' => b'\n',
            b'r' => b'\r',
            b't' => b'\t',
            b'v' => 0x0b,
            other => other,
        };
        out.push(byte);
    }
    out
}
