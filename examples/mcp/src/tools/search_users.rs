use hephos::prelude::*;

use crate::domain::user::{SearchQuery, UserSummary};
use crate::services::directory::Directory;

/// Identical in shape to `example-tools`: one tool definition, now exposed over
/// MCP instead of to an in-process agent.
pub struct SearchUsers {
    directory: Directory,
}

impl SearchUsers {
    pub fn new(directory: Directory) -> Self {
        SearchUsers { directory }
    }
}

impl Tool for SearchUsers {
    type Input = SearchQuery;
    type Output = Vec<UserSummary>;
    const NAME: &'static str = "search_users";
    const DESCRIPTION: &'static str = "Search the user directory by name or email.";

    async fn call(&self, input: SearchQuery) -> Result<Vec<UserSummary>> {
        if input.query.trim().is_empty() {
            return Err(Error::invalid("query must not be empty"));
        }
        self.directory.search(&input.query).await
    }
}
