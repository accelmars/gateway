//! Streaming chat completion via AccelMars Gateway.
//!
//! Parses SSE chunks (data: <json>\n\n) and prints each delta as it arrives.
//!
//! Prerequisites:
//!   gateway serve  (or GATEWAY_MODE=mock gateway serve)
//!
//! Run:
//!   cargo run --bin streaming
//!   ACCELMARS_GATEWAY_URL=http://localhost:8080 cargo run --bin streaming

use futures::StreamExt;
use serde::{Deserialize, Serialize};
use std::env;
use std::io::Write;

#[derive(Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<Message>,
    stream: bool,
}

#[derive(Serialize)]
struct Message {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct StreamChunk {
    choices: Vec<StreamChoice>,
}

#[derive(Deserialize)]
struct StreamChoice {
    delta: Delta,
}

#[derive(Deserialize)]
struct Delta {
    content: Option<String>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let gateway_url = env::var("ACCELMARS_GATEWAY_URL")
        .unwrap_or_else(|_| "http://localhost:4000".to_string());

    let client = reqwest::Client::new();

    let request = ChatRequest {
        model: "standard".to_string(),
        messages: vec![Message {
            role: "user".to_string(),
            content: "Count to 5, one number per line.".to_string(),
        }],
        stream: true,
    };

    let response = client
        .post(format!("{}/v1/chat/completions", gateway_url))
        .header("Authorization", "Bearer local")
        .json(&request)
        .send()
        .await?
        .error_for_status()?;

    let mut stream = response.bytes_stream();
    let mut buffer = String::new();

    'outer: while let Some(chunk) = stream.next().await {
        let bytes: bytes::Bytes = chunk?;
        buffer.push_str(&String::from_utf8_lossy(&bytes));

        loop {
            match buffer.find('\n') {
                None => break,
                Some(pos) => {
                    let line = buffer[..pos].trim_end_matches('\r').to_string();
                    buffer = buffer[pos + 1..].to_string();

                    if !line.starts_with("data: ") {
                        continue;
                    }

                    let data = &line["data: ".len()..];

                    if data == "[DONE]" {
                        break 'outer;
                    }

                    if let Ok(chunk_json) = serde_json::from_str::<StreamChunk>(data) {
                        if let Some(content) = &chunk_json.choices[0].delta.content {
                            print!("{}", content);
                            std::io::stdout().flush()?;
                        }
                    }
                }
            }
        }
    }

    println!();
    Ok(())
}
