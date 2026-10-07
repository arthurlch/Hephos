use uuid::Uuid;

/// Who is making the request.
///
/// Authentication middleware resolves an `Identity` and inserts it into the
/// request. [`crate::Ctx`] exposes it. There are exactly two states: an
/// anonymous caller, or an authenticated [`Principal`].
///
/// Rivet deliberately keeps the principal small — an id plus roles. Loading a
/// full user record is a service concern, done by id when needed. This keeps the
/// auth layer thin and uniform across every app.
#[derive(Debug, Clone)]
pub enum Identity {
    Anonymous,
    User(Principal),
}

/// The authenticated caller.
#[derive(Debug, Clone)]
pub struct Principal {
    pub id: Uuid,
    pub roles: Vec<String>,
}

impl Principal {
    pub fn has_role(&self, role: &str) -> bool {
        self.roles.iter().any(|r| r == role)
    }
}

impl Default for Identity {
    fn default() -> Self {
        Identity::Anonymous
    }
}
