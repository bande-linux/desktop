use std::io::{Read, Write};

use bande_pam::Conversation;
use zeroize::Zeroizing;

use crate::protocol::{FromWorker, ToWorker, read_message, write_message};

const TEXT_LIMIT: usize = 4096;

fn clipped(text: &str) -> String {
    let text = text.trim();
    match text.char_indices().nth(TEXT_LIMIT) {
        Some((end, _)) => text[..end].to_owned(),
        None => text.to_owned(),
    }
}

/// Carries a PAM conversation between a session worker and `bande-login`.
pub struct Relay<S> {
    stream: S,
}

impl<S: Read + Write> Relay<S> {
    pub fn new(stream: S) -> Self {
        Self { stream }
    }

    fn tell(&mut self, said: &FromWorker) -> bool {
        write_message(&mut self.stream, said)
            .inspect_err(|error| tracing::warn!("cannot reach bande-login: {error}"))
            .is_ok()
    }
}

impl<S: Read + Write> Conversation for Relay<S> {
    fn ask(&mut self, prompt: &str, visible: bool) -> Option<Zeroizing<String>> {
        let asked = FromWorker::Ask {
            text: clipped(prompt),
            visible,
        };
        if !self.tell(&asked) {
            return None;
        }
        match read_message(&mut self.stream) {
            Ok(ToWorker::Answer { response }) => response,
            Ok(other) => {
                tracing::warn!("bande-login sent {other:?} instead of an answer");
                None
            }
            Err(error) => {
                tracing::info!("the conversation ended: {error}");
                None
            }
        }
    }

    fn show(&mut self, message: &str, error: bool) {
        self.tell(&FromWorker::Show {
            text: clipped(message),
            error,
        });
    }
}

/// The conversation of a session nobody signs in to: every prompt is refused.
pub struct Unattended;

impl Conversation for Unattended {
    fn ask(&mut self, prompt: &str, _visible: bool) -> Option<Zeroizing<String>> {
        tracing::warn!("the login window's PAM stack asked {prompt:?}; refusing");
        None
    }

    fn show(&mut self, message: &str, _error: bool) {
        tracing::info!("the login window's PAM stack says {message:?}");
    }
}
