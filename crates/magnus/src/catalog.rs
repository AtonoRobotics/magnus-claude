//! The connector catalog: the manifests of the connectors that ship in
//! Magnus (`catalog/*.toml`), compiled into the appliance so the release's
//! signature covers them (connectors spec, "Catalog and connector
//! manifests"). A manifest's tool classes count as the organisation owner's
//! classification (owner's ruling 5). A connector whose provider needs
//! Alpha Vector's own client is in no one's Library until the owner sets
//! that registration (`/v1/registrations`).

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

/// The providers whose registration the owner sets: those of the
/// catalog's `preregistered` connectors.
pub fn providers() -> Vec<String> {
    let mut p: Vec<String> = manifests().iter().filter(|m| m.registration == "preregistered").filter_map(|m| m.provider.clone()).collect();
    p.sort();
    p.dedup();
    p
}

/// The vault name of the organisation's registration with a provider.
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
            assert!(!m.summary.to_lowercase().contains("mcp") && !m.summary.to_lowercase().contains("oauth"), "{}: protocol words", m.id);
        }
    }
}
