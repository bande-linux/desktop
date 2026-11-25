use bande_l10n::tr;

use crate::{Effect, Refusal};

pub(crate) struct Question<'a> {
    pub(crate) agent: Option<&'a str>,
    pub(crate) effects: &'a [Effect],
}

impl Question<'_> {
    fn is_asked(&self) -> bool {
        self.effects.iter().any(|effect| match effect {
            Effect::Read | Effect::ReadForeign | Effect::Show | Effect::Change => false,
            Effect::Disrupt => self.agent.is_some(),
            Effect::Send | Effect::Critical => true,
        })
    }
}

pub(crate) fn allow(question: &Question<'_>) -> Result<(), Refusal> {
    if !question.is_asked() {
        return Ok(());
    }
    Err(Refusal::unavailable(tr!(
        "This has to be allowed first, and there is nothing to ask with yet"
    )))
}
