//! The messages between the launcher and an auth worker, one per packet on
//! a `SOCK_SEQPACKET` socketpair: a tag byte, then the payload.

use std::time::Duration;

/// How an attempt ended. The discriminant is its byte on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Outcome {
    Success = 0,
    /// The answers were wrong, or the account refused.
    Failed = 1,
    /// This method can't authenticate this user here: no PIN set, no
    /// such service, and so on.
    Unavailable = 2,
    /// Something broke: the worker, the conversation or PAM itself.
    Error = 3,
}

impl Outcome {
    /// Every outcome, in the order of their bytes.
    const ALL: [Outcome; 4] = [
        Outcome::Success,
        Outcome::Failed,
        Outcome::Unavailable,
        Outcome::Error,
    ];
}

/// What a worker says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FromWorker {
    /// A question to answer, shown (`secret: false`) or hidden as typed.
    Prompt {
        secret: bool,
        text: String,
    },
    Info(String),
    Error(String),
    /// A failure delay started: answers wait this long.
    Delay(Duration),
    Done(Outcome),
}

const PROMPT_SECRET: u8 = b'P';
const PROMPT_SHOWN: u8 = b'p';
const INFO: u8 = b'I';
const ERROR: u8 = b'E';
const DELAY: u8 = b'D';
const DONE: u8 = b'R';
const ANSWER: u8 = b'A';

/// The largest packet either side sends or reads.
pub const MAX_PACKET: usize = 4096;

impl FromWorker {
    pub fn encode(&self) -> Vec<u8> {
        let (tag, payload): (u8, &[u8]) = match self {
            FromWorker::Prompt { secret: true, text } => (PROMPT_SECRET, text.as_bytes()),
            FromWorker::Prompt {
                secret: false,
                text,
            } => (PROMPT_SHOWN, text.as_bytes()),
            FromWorker::Info(text) => (INFO, text.as_bytes()),
            FromWorker::Error(text) => (ERROR, text.as_bytes()),
            FromWorker::Delay(d) => {
                let us = u32::try_from(d.as_micros()).unwrap_or(u32::MAX);
                return [&[DELAY][..], &us.to_le_bytes()].concat();
            }
            FromWorker::Done(o) => return vec![DONE, *o as u8],
        };
        let mut out = Vec::with_capacity(1 + payload.len());
        out.push(tag);
        out.extend_from_slice(&payload[..payload.len().min(MAX_PACKET - 1)]);
        out
    }

    pub fn decode(packet: &[u8]) -> Option<FromWorker> {
        let (&tag, rest) = packet.split_first()?;
        let text = || String::from_utf8_lossy(rest).into_owned();
        Some(match tag {
            PROMPT_SECRET => FromWorker::Prompt {
                secret: true,
                text: text(),
            },
            PROMPT_SHOWN => FromWorker::Prompt {
                secret: false,
                text: text(),
            },
            INFO => FromWorker::Info(text()),
            ERROR => FromWorker::Error(text()),
            DELAY => {
                let us = u32::from_le_bytes(rest.try_into().ok()?);
                FromWorker::Delay(Duration::from_micros(us.into()))
            }
            DONE => match rest {
                [byte] => FromWorker::Done(*Outcome::ALL.get(usize::from(*byte))?),
                _ => return None,
            },
            _ => return None,
        })
    }
}

/// The answer to the last prompt.
pub fn encode_answer(answer: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(answer.len() + 1);
    out.push(ANSWER);
    out.extend_from_slice(&answer[..answer.len().min(MAX_PACKET - 1)]);
    out
}

/// The answer in a packet from the launcher.
pub fn decode_answer(packet: &[u8]) -> Option<&[u8]> {
    match packet.split_first()? {
        (&ANSWER, rest) => Some(rest),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_message_round_trips() {
        for m in [
            FromWorker::Prompt {
                secret: true,
                text: "PIN".into(),
            },
            FromWorker::Prompt {
                secret: false,
                text: "Code".into(),
            },
            FromWorker::Info("The account is locked due to 3 failed logins.".into()),
            FromWorker::Error("Authentication failure".into()),
            FromWorker::Delay(Duration::from_secs(2)),
            FromWorker::Done(Outcome::Success),
            FromWorker::Done(Outcome::Failed),
            FromWorker::Done(Outcome::Unavailable),
            FromWorker::Done(Outcome::Error),
        ] {
            assert_eq!(FromWorker::decode(&m.encode()), Some(m));
        }
    }

    #[test]
    fn junk_does_not_decode() {
        assert_eq!(FromWorker::decode(&[]), None);
        assert_eq!(FromWorker::decode(b"Zx"), None);
        assert_eq!(FromWorker::decode(&[DELAY, 1, 2]), None);
        assert_eq!(FromWorker::decode(&[DONE, 9]), None);
    }

    #[test]
    fn an_answer_round_trips() {
        assert_eq!(decode_answer(&encode_answer(b"1234")), Some(&b"1234"[..]));
        assert_eq!(decode_answer(b"P1234"), None);
    }
}
