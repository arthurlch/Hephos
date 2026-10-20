use hephos::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CalcInput {
    pub expression: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CalcOutput {
    pub result: f64,
}

#[derive(Clone, Default)]
pub struct CalculatorTool;

impl Tool for CalculatorTool {
    type Input = CalcInput;
    type Output = CalcOutput;
    const NAME: &'static str = "calculate";
    const DESCRIPTION: &'static str =
        "Perform mathematical calculation in '<a > <op> <b >' format.";

    async fn call(&self, input: Self::Input) -> Result<Self::Output> {
        let parts: Vec<&str> = input.expression.split_whitespace().collect();
        if parts.len() != 3 {
            return Err(Error::invalid(
                "expected expression: '<number> <op> <number>'",
            ));
        }
        let a: f64 = parts[0]
            .parse()
            .map_err(|_| Error::invalid("invalid first number"))?;
        let op = parts[1];
        let b: f64 = parts[2]
            .parse()
            .map_err(|_| Error::invalid("invalid second number"))?;

        let res = match op {
            "+" => a + b,
            "-" => a - b,
            "*" => a * b,
            "/" => {
                if b == 0.0 {
                    return Err(Error::invalid("division by zero"));
                }
                a / b
            }
            _ => return Err(Error::invalid("unknown operator")),
        };

        Ok(CalcOutput { result: res })
    }
}
