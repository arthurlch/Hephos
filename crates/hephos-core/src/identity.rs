use uuid::Uuid;

/// Who is making the request.
///
/// Authentication middleware resolves an `Identity` and inserts it into the
/// request. [`crate::Ctx`] exposes it. There are exactly two states: an
/// anonymous caller, or an authenticated [`Principal`].
///
/// Hephos deliberately keeps the principal small — an id plus roles. Loading a
/// full user record is a service concern, done by id when needed. This keeps the
/// auth layer thin and uniform across every app.
#[derive(Debug, Clone, Default)]
pub enum Identity {
    #[default]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_identity_is_anonymous() {
        assert!(matches!(Identity::default(), Identity::Anonymous));
    }

    #[test]
    fn has_role_checks_membership() {
        let p = Principal {
            id: Uuid::nil(),
            roles: vec!["admin".into(), "user".into()],
        };
        assert!(p.has_role("admin"));
        assert!(!p.has_role("owner"));
    }
}
