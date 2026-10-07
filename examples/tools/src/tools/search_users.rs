use rivet::prelude::*;

use crate::domain::user::{SearchQuery, UserSummary};
use crate::services::directory::Directory;

/// A tool captures its dependencies and the caller's identity at construction, so
/// authorization happens where the identity is known. The agent layer never needs
/// to know about any of this.
pub struct SearchUsers {
    directory: Directory,
    caller: Identity,
}

impl SearchUsers {
    pub fn new(directory: Directory, caller: Identity) -> Self {
        SearchUsers { directory, caller }
    }

    fn authorize(&self) -> Result<()> {
        match &self.caller {
            Identity::User(principal) if principal.has_role("support") => Ok(()),
            Identity::User(_) => Err(Error::Forbidden),
            Identity::Anonymous => Err(Error::Unauthorized),
        }
    }
}

impl Tool for SearchUsers {
    type Input = SearchQuery;
    type Output = Vec<UserSummary>;
    const NAME: &'static str = "search_users";
    const DESCRIPTION: &'static str = "Search the user directory by name or email.";

    async fn call(&self, input: SearchQuery) -> Result<Vec<UserSummary>> {
        self.authorize()?;
        if input.query.trim().is_empty() {
            return Err(Error::invalid("query must not be empty"));
        }
        self.directory.search(&input.query).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn tool(roles: Vec<String>) -> SearchUsers {
        let caller = Identity::User(Principal { id: Uuid::nil(), roles });
        SearchUsers::new(Directory::load(), caller)
    }

    #[tokio::test]
    async fn call_without_role_is_forbidden() {
        let err = tool(vec![])
            .call(SearchQuery { query: "ada".into() })
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Forbidden));
    }

    #[tokio::test]
    async fn call_rejects_empty_query() {
        let err = tool(vec!["support".into()])
            .call(SearchQuery { query: "  ".into() })
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Invalid(_)));
    }

    #[tokio::test]
    async fn call_returns_matches() {
        let found = tool(vec!["support".into()])
            .call(SearchQuery { query: "turing".into() })
            .await
            .unwrap();
        assert_eq!(found.len(), 1);
    }
}
