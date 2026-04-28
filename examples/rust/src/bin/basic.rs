//! Basic chat completion via AccelMars Gateway.
//!
//! Prerequisites:
//!   gateway serve  (or GATEWAY_MODE=mock gateway serve)
//!
//! Run:
//!   cargo run --bin basic
//!   ACCELMARS_GATEWAY_URL=http://localhost:8080 cargo run --bin basic

use serde::{Deserialize, Serialize};
use std::env;

#[derive(Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<Message>,
}

#[derive(Serialize, Deserialize)]
struct Message {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct ChatResponse {
    model: String,
    choices: Vec<Choice>,
    usage: Usage,
}

#[derive(Deserialize)]
struct Choice {
    message: Message,
}

#[derive(Deserialize)]
struct Usage {
    total_tokens: u32,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let gateway_url = env::var("ACCELMARS_GATEWAY_URL")
        .unwrap_or_else(|_| "http://localhost:4000".to_string());

    let client = reqwest::Client::new();

    let request = ChatRequest {
        model: "standard".to_string(),
        messages: vec![
            Message {
                role: "system".to_string(),
                content: "You are a helpful assistant.".to_string(),
            },
            Message {
                role: "user".to_string(),
                content: "What is 2 + 2?".to_string(),
            },
        ],
    };

    let response = client
        .post(format!("{}/v1/chat/completions", gateway_url))
        .header("Authorization", "Bearer local")
        .json(&request)
        .send()
        .await?
        .error_for_status()?
        .json::<ChatResponse>()
        .await?;

    println!("{}", response.choices[0].message.content);
    println!("\nModel: {}  |  Tokens: {}", response.model, response.usage.total_tokens);

    Ok(())
}
