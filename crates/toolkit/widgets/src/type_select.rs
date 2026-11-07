use std::time::{Duration, Instant};

const PAUSE: Duration = Duration::from_secs(1);

#[derive(Debug, Default)]
pub struct TypeSelect {
    typed: String,
    last: Option<Instant>,
}

impl TypeSelect {
    pub fn push(&mut self, characters: impl IntoIterator<Item = char>, now: Instant) -> &str {
        if self
            .last
            .is_some_and(|last| now.saturating_duration_since(last) > PAUSE)
        {
            self.typed.clear();
        }
        self.last = Some(now);
        self.typed
            .extend(characters.into_iter().flat_map(char::to_lowercase));
        &self.typed
    }

    pub fn clear(&mut self) {
        self.typed.clear();
    }
}
