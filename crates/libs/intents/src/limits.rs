use std::collections::{BTreeMap, HashMap};

use bande_l10n::tr;
use zbus::zvariant::{self, OwnedValue};

use crate::plan::put;
use crate::{Dict, Effect, Reason, Refusal};

/// What the person has let a caller do, as the broker passes it on; a program of the person's own
/// has none.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Limits {
    agent: Option<String>,
    domains: Option<BTreeMap<String, Grant>>,
}

#[derive(Debug, Clone, PartialEq)]
struct Grant {
    effects: Vec<Effect>,
    within: Option<Vec<String>>,
}

fn invalid(name: &str) -> Refusal {
    Refusal::invalid(name, vec!["agent".to_owned(), "domains".to_owned()])
}

impl Limits {
    pub(crate) fn decode(mut limits: Dict) -> Result<Self, Refusal> {
        let agent = limits
            .remove("agent")
            .map(String::try_from)
            .transpose()
            .map_err(|_| invalid("agent"))?;
        let domains = limits
            .remove("domains")
            .map(HashMap::<String, HashMap<String, OwnedValue>>::try_from)
            .transpose()
            .map_err(|_| invalid("domains"))?
            .map(|domains| {
                domains
                    .into_iter()
                    .map(|(domain, grant)| Ok((domain, Grant::decode(grant)?)))
                    .collect::<Result<_, Refusal>>()
            })
            .transpose()?;
        match limits.keys().next() {
            Some(name) => Err(invalid(name)),
            None => Ok(Self { agent, domains }),
        }
    }

    /// What to pass on when an owner asks another one: the same, whole.
    pub fn encode(&self) -> Result<Dict, Refusal> {
        let mut limits = Dict::new();
        if let Some(agent) = &self.agent {
            put(&mut limits, "agent", agent.as_str())?;
        }
        if let Some(domains) = &self.domains {
            let domains = domains
                .iter()
                .map(|(domain, grant)| Ok((domain.clone(), grant.encode()?)))
                .collect::<Result<HashMap<String, Dict>, Refusal>>()?;
            put(&mut limits, "domains", domains)?;
        }
        Ok(limits)
    }

    /// The registered name of the agent this is done for.
    pub fn agent(&self) -> Option<&str> {
        self.agent.as_deref()
    }

    fn grant(&self, domain: &str) -> Result<Option<&Grant>, Refusal> {
        match &self.domains {
            None => Ok(None),
            Some(domains) => domains
                .get(domain)
                .map(Some)
                .ok_or_else(|| not_granted(domain)),
        }
    }

    /// Whether the caller may use `effect` in `domain`; showing comes with reading.
    pub fn allows(&self, domain: &str, effect: Effect) -> bool {
        match self.grant(domain) {
            Err(_) => false,
            Ok(None) => true,
            Ok(Some(grant)) => {
                grant.effects.contains(&effect)
                    || (effect == Effect::Show && grant.effects.contains(&Effect::Read))
            }
        }
    }

    /// The containers of `domain` the caller is kept to; `None` is all of them.
    pub fn within(&self, domain: &str) -> Option<&[String]> {
        self.grant(domain)
            .ok()
            .flatten()
            .and_then(|grant| grant.within.as_deref())
    }

    /// Whether `container`, a reference or word of `domain`, is one the caller may reach.
    pub fn covers(&self, domain: &str, container: &str) -> bool {
        match self.grant(domain) {
            Err(_) => false,
            Ok(grant) => grant
                .and_then(|grant| grant.within.as_ref())
                .is_none_or(|within| within.iter().any(|allowed| allowed == container)),
        }
    }

    pub(crate) fn check(&self, domain: &str, effects: &[Effect]) -> Result<(), Refusal> {
        self.grant(domain)?;
        match effects.iter().all(|effect| self.allows(domain, *effect)) {
            true => Ok(()),
            false => Err(not_granted(domain)),
        }
    }

    pub(crate) fn grants(&self, domain: &str) -> bool {
        self.grant(domain).is_ok()
    }
}

fn not_granted(domain: &str) -> Refusal {
    Refusal::new(
        Reason::NotGranted,
        tr!("{domain} is not permitted", domain = domain),
    )
}

impl Grant {
    fn encode(&self) -> Result<Dict, Refusal> {
        let mut grant = Dict::new();
        let effects: Vec<&str> = self.effects.iter().map(|effect| effect.word()).collect();
        put(&mut grant, "effects", effects)?;
        if let Some(within) = &self.within {
            put(&mut grant, "within", within.clone())?;
        }
        Ok(grant)
    }

    fn decode(mut grant: HashMap<String, OwnedValue>) -> Result<Self, Refusal> {
        let mut list = |name: &str| {
            grant
                .remove(name)
                .map(Vec::<String>::try_from)
                .transpose()
                .map_err(|_: zvariant::Error| invalid(name))
        };
        let effects = list("effects")?
            .unwrap_or_default()
            .iter()
            .map(|word| Effect::from_word(word).ok_or_else(|| invalid("effects")))
            .collect::<Result<_, _>>()?;
        let within = list("within")?;
        match grant.keys().next() {
            Some(name) => Err(invalid(name)),
            None => Ok(Self { effects, within }),
        }
    }
}
