use serde::{Deserialize, Serialize};
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
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Identity {
    #[default]
    Anonymous,
    User(Principal),
}

impl Identity {
    /// Create an authenticated [`Identity::User`] with an id and roles.
    pub fn user(id: Uuid, roles: Vec<String>) -> Self {
        Identity::User(Principal::new(id, roles))
    }

    /// Returns `true` if the identity is an authenticated user.
    pub fn is_authenticated(&self) -> bool {
        matches!(self, Identity::User(_))
    }

    /// Returns `true` if the identity is anonymous.
    pub fn is_anonymous(&self) -> bool {
        matches!(self, Identity::Anonymous)
    }

    /// Return a reference to the [`Principal`] if authenticated, or `None` if anonymous.
    pub fn principal(&self) -> Option<&Principal> {
        match self {
            Identity::User(p) => Some(p),
            Identity::Anonymous => None,
        }
    }
}

/// The authenticated caller.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Principal {
    pub id: Uuid,
    pub roles: Vec<String>,
}

impl Principal {
    /// Construct a new [`Principal`] with the given id and roles.
    pub fn new(id: Uuid, roles: Vec<String>) -> Self {
        Principal { id, roles }
    }

    /// Returns `true` if the principal holds the given role.
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
        // Empty role does not match
        assert!(!p.has_role(""));
        // Case-sensitive match
        assert!(!p.has_role("Admin"));
    }

    #[test]
    fn principal_with_empty_roles() {
        let p = Principal {
            id: Uuid::new_v4(),
            roles: vec![],
        };
        assert!(!p.has_role("admin"));
        assert!(!p.has_role("user"));
    }

    #[test]
    fn identity_cloning_and_debug_format() {
        let id = Uuid::new_v4();
        let user_id = Identity::User(Principal {
            id,
            roles: vec!["operator".into()],
        });
        let cloned = user_id.clone();
        if let Identity::User(principal) = cloned {
            assert_eq!(principal.id, id);
            assert!(principal.has_role("operator"));
        } else {
            panic!("expected Identity::User");
        }

        let debug_str = format!("{:?}", user_id);
        assert!(debug_str.contains("operator"));

        let anon = Identity::Anonymous;
        assert_eq!(format!("{:?}", anon), "Anonymous");
    }
}
