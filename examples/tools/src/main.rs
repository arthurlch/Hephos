mod domain;
mod services;
mod tools;

use hephos::prelude::*;
use uuid::Uuid;

use crate::domain::user::SearchQuery;
use crate::services::directory::Directory;
use crate::tools::search_users::SearchUsers;

#[tokio::main]
async fn main() -> Result<()> {
    let directory = Directory::load();
    let caller = Identity::User(Principal {
        id: Uuid::nil(),
        roles: vec!["support".into()],
    });

    let tool = SearchUsers::new(directory, caller);
    let matches = tool
        .call(SearchQuery {
            query: "ada".into(),
        })
        .await?;

    println!("{} match(es)", matches.len());
    for user in matches {
        println!("- {} <{}>", user.name, user.email);
    }
    Ok(())
}
