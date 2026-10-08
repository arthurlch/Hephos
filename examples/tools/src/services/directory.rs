use std::sync::Arc;

use hephos::prelude::*;
use uuid::Uuid;

use crate::domain::user::UserSummary;

/// An immutable user directory. Loaded once, shared lock-free.
#[derive(Clone)]
pub struct Directory {
    users: Arc<[UserSummary]>,
}

impl Directory {
    pub fn load() -> Self {
        let users = seed().into();
        Directory { users }
    }

    pub async fn search(&self, query: &str) -> Result<Vec<UserSummary>> {
        let needle = query.to_lowercase();
        let found = self
            .users
            .iter()
            .filter(|u| {
                u.name.to_lowercase().contains(&needle) || u.email.to_lowercase().contains(&needle)
            })
            .cloned()
            .collect();
        Ok(found)
    }
}

fn seed() -> Vec<UserSummary> {
    vec![
        UserSummary {
            id: Uuid::from_u128(1),
            name: "Ada Lovelace".into(),
            email: "ada@example.com".into(),
        },
        UserSummary {
            id: Uuid::from_u128(2),
            name: "Alan Turing".into(),
            email: "alan@example.com".into(),
        },
    ]
}
