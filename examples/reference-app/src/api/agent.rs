use hephos::prelude::*;

use crate::domain::assistant::{AskRequest, AskResponse};
use crate::state::{AppState, Ctx};

pub fn routes() -> Router<AppState> {
    Router::new().post("/assistant/ask", ask)
}

pub async fn ask(ctx: Ctx, Json(input): Json<AskRequest>) -> Result<Json<AskResponse>> {
    let answer = ctx.state().assistant.ask(&ctx, &input.prompt).await?;
    Ok(Json(AskResponse { answer }))
}
