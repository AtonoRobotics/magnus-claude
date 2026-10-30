//! The identity interface: how people prove who they are, through whichever
//! identity provider the appliance uses. An OpenID Connect provider
//! (`oidc.rs`; Keycloak for one) is the one built; SAML is to sign in the
//! same way, in the person's browser, and SCIM to feed the same people and
//! groups (docs/proposals/accounts.md). Every decision about a person still
//! reads `Kernel::person`, whichever provider signed them in.

use anyhow::Result;

/// Groups and roles the provider gave, where the owner named where they
/// are; None where not.
#[derive(Debug, Default, Clone)]
pub struct Rights {
    pub groups: Option<Vec<String>>,
    pub roles: Option<Vec<String>>,
}

/// Where to send the person's browser, and for how long the sign-in waits.
pub struct Begun {
    pub state: String,
    pub authorize_url: String,
    pub expires_in: i64,
}

pub enum Polled {
    /// Not finished yet; ask again later.
    Pending,
    /// Signed in: who, whether in the app, and their rights.
    Done { person: String, app: bool, rights: Rights },
    /// Over: declined, expired or refused, with a message for the person.
    Failed(String),
}

#[async_trait::async_trait]
pub trait Identity: Send + Sync {
    /// What the sign-in button names.
    fn label(&self) -> String;
    /// Begin a sign-in in the person's own browser, which comes back to
    /// `redirect_uri`, a loopback address on their computer.
    async fn begin(&self, redirect_uri: &str, app: bool) -> Result<Begun>;
    /// Finish it with what came back: the code, the state it answers, and
    /// who sent it back, if the provider says.
    async fn finish(&self, state: &str, code: &str, iss: Option<&str>) -> Result<Polled>;
}
