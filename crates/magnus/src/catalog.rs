//! The connector catalog: the manifests of the connectors that ship in
//! Magnus (`catalog/*.toml`), compiled into the appliance so the release's
//! signature covers them (connectors spec, "Catalog and connector
//! manifests"). A manifest's tool classes count as the organisation owner's
//! classification (owner's ruling 5). Every catalog connector is in every
//! Library, already set up: no one, person or admin, is asked for a client
//! ID, secret or address (connectors spec rev 46, "What the user sees").
//! A connector whose provider needs Alpha Vector's own client signs in
//! through it (`catalog/registrations.toml`); its secret, when it has one,
//! is Alpha Vector's, provisioned to the appliance until the relay holds it.

use crate::config::{Connector, ToolClass};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::sync::OnceLock;

const SOURCES: &[&str] = &[
    include_str!("../../../catalog/google-gmail.toml"),
    include_str!("../../../catalog/google-calendar.toml"),
    include_str!("../../../catalog/google-drive.toml"),
    include_str!("../../../catalog/google-chat.toml"),
];

#[derive(Deserialize, Clone, Debug)]
pub struct Manifest {
    pub id: String,
    pub version: String,
    pub name: String,
    pub summary: String,
    /// The Library's card and page (connectors spec, Catalog and connector
    /// manifests): who makes it ("by …"), its category chip, what Magnus can
    /// do with it, each scope in plain words, and what it can and cannot do
    /// here. `letters` and `tint` draw its tile until the provider's own mark
    /// is in its listing.
    pub publisher: String,
    pub category: String,
    pub abilities: Vec<String>,
    pub asks: Vec<String>,
    pub can: String,
    pub cannot: String,
    pub letters: String,
    pub tint: String,
    pub kind: String,
    pub endpoint: String,
    pub audience: String,
    pub label: String,
    /// `dcr` (Magnus registers itself) or `preregistered` (Alpha Vector's
    /// own client with `provider`).
    pub registration: String,
    #[serde(default)]
    pub provider: Option<String>,
    /// Exactly the access asked for at sign-in.
    #[serde(default)]
    pub scopes: Vec<String>,
    /// Every tool's class.
    #[serde(default)]
    pub tools: BTreeMap<String, String>,
}

pub fn manifests() -> &'static [Manifest] {
    static M: OnceLock<Vec<Manifest>> = OnceLock::new();
    M.get_or_init(|| SOURCES.iter().map(|s| toml::from_str(s).expect("a catalog manifest parses (checked by a unit test)")).collect())
}

/// The Library's category chips, in their order.
pub const CATEGORIES: [&str; 5] = ["email", "files", "code", "work", "design"];

pub fn manifest(id: &str) -> Option<&'static Manifest> {
    manifests().iter().find(|m| m.id == id)
}

/// Alpha Vector's own client with a provider.
#[derive(Deserialize, Clone, Debug)]
pub struct Client {
    pub client_id: String,
    /// `desktop` or `confidential` (a secret at the token exchange), or
    /// `public` (none).
    pub kind: String,
}

impl Client {
    pub fn needs_secret(&self) -> bool {
        self.kind != "public"
    }
}

/// Alpha Vector's client with `provider`, if it has one.
pub fn client(provider: &str) -> Option<&'static Client> {
    static C: OnceLock<BTreeMap<String, Client>> = OnceLock::new();
    C.get_or_init(|| toml::from_str(include_str!("../../../catalog/registrations.toml")).expect("the catalog's registrations parse (checked by a unit test)")).get(provider)
}

/// The vault name of Alpha Vector's provisioned secret for a provider.
pub fn registration_name(provider: &str) -> String {
    format!("registration_{provider}")
}

/// A manifest as one of the organisation's apps. Its client ID is read
/// from the registration at sign-in, never kept here.
pub fn connector(m: &Manifest) -> Connector {
    Connector {
        name: m.id.clone(),
        transport: "mcp_http".into(),
        url: m.endpoint.clone(),
        credential: Some(crate::connectors::credential_name(&m.id)),
        audience: m.audience.clone(),
        label: m.label.clone(),
        access_groups: vec![],
        idempotency: "none".into(),
        tools: m.tools.iter().map(|(t, risk)| (t.clone(), ToolClass { risk: risk.clone(), label: None })).collect(),
        auth: Some(crate::connectors::OAUTH.into()),
        client_id: None,
        system_of_record: false,
        scopes: m.scopes.clone(),
        registration: m.provider.clone().filter(|_| m.registration == "preregistered"),
        title: Some(m.name.clone()),
        summary: Some(m.summary.clone()),
        catalog: true,
    }
}

#[cfg(test)]
mod tests {
    /// Every manifest parses, names a known kind, registration, audience,
    /// label and tool class, and a preregistered one names its provider.
    #[test]
    fn every_manifest_is_well_formed() {
        let ms = super::manifests();
        assert!(!ms.is_empty());
        let mut ids = std::collections::HashSet::new();
        for m in ms {
            assert!(ids.insert(m.id.clone()), "{} twice", m.id);
            assert!(!m.id.contains("__") && m.id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'), "{}", m.id);
            assert_eq!(m.kind, "remote", "{}", m.id);
            assert!(m.endpoint.starts_with("https://"), "{}", m.id);
            assert!(["dcr", "preregistered"].contains(&m.registration.as_str()), "{}", m.id);
            assert_eq!(m.registration == "preregistered", m.provider.is_some(), "{}", m.id);
            assert!(crate::tools::Audience::parse(&m.audience).is_some(), "{}", m.id);
            assert!(crate::tools::Label::parse(&m.label).is_some(), "{}", m.id);
            for (t, risk) in &m.tools {
                assert!(["read", "reversible_write", "irreversible", "financial"].contains(&risk.as_str()), "{}: {t} is {risk}", m.id);
            }
            // What the Library shows: a category chip, abilities, a plain line
            // for every scope asked for, and never a protocol word.
            assert!(super::CATEGORIES.contains(&m.category.as_str()), "{}: category {}", m.id, m.category);
            assert!(!m.publisher.is_empty() && !m.abilities.is_empty() && !m.can.is_empty() && !m.cannot.is_empty(), "{}: its listing is incomplete", m.id);
            assert_eq!(m.asks.len(), m.scopes.len(), "{}: one plain line for each scope asked for", m.id);
            assert!((1..=2).contains(&m.letters.chars().count()) && m.tint.starts_with('#') && m.tint.len() == 7, "{}: its tile", m.id);
            let shown = [&m.summary, &m.can, &m.cannot].into_iter().chain(&m.abilities).chain(&m.asks).map(|s| s.to_lowercase()).collect::<Vec<_>>();
            for word in ["mcp", "oauth", "scope", "token", "api", "port"] {
                assert!(shown.iter().all(|s| !s.split(|c: char| !c.is_alphanumeric()).any(|w| w == word)), "{}: protocol word {word:?} in what the Library shows", m.id);
            }
            // A preregistered connector is set up already: its provider's client is in the catalog.
            if let Some(p) = &m.provider {
                assert!(super::client(p).is_some(), "{}: no client for {p} in catalog/registrations.toml", m.id);
            }
        }
        for p in ["google", "microsoft", "github", "vercel", "slack", "sentry"] {
            let c = super::client(p).unwrap_or_else(|| panic!("{p}"));
            assert!(!c.client_id.is_empty() && ["desktop", "confidential", "public"].contains(&c.kind.as_str()), "{p}: {c:?}");
        }
    }
}
