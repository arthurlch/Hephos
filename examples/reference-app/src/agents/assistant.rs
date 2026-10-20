use std::time::Duration;

use hephos::agent::{Agent, Limits, Model};

use crate::tools::CalculatorTool;

pub fn build<M: Model>(model: M) -> Agent<M> {
    Agent::new(model)
        .system("You are an intelligent assistant with access to tools.")
        .tool(CalculatorTool)
        .limits(Limits {
            max_turns: 5,
            max_tokens: 1024,
            timeout: Duration::from_secs(30),
            max_tool_calls_per_turn: 10,
        })
}
