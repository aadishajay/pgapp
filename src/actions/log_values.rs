//! A minimal demo module showing what a custom Rust action looks like:
//! logs the request's parameter map to the server's stdout and reports
//! how many values it saw. Real modules do real work — anything Rust
//! and sqlx can do — with this same shape.

use crate::actions::{ActionContext, BoxFuture, ServerAction};

pub struct LogValues;

impl ServerAction for LogValues {
    fn name(&self) -> &'static str {
        "log_values"
    }

    fn run<'a>(&'a self, ctx: ActionContext<'a>) -> BoxFuture<'a, anyhow::Result<String>> {
        Box::pin(async move {
            let mut keys: Vec<&String> = ctx.values.keys().collect();
            keys.sort();
            for key in &keys {
                println!("[log_values] {} = {}", key, ctx.values[*key]);
            }
            Ok(format!("Logged {} value(s) to the server console.", keys.len()))
        })
    }
}
