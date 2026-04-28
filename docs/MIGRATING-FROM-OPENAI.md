# Migrating from OpenAI SDK

> AccelMars Gateway speaks the OpenAI API. No new SDK needed — swap the base URL and you're done.

---

## The one-line change

```python
# Before
client = OpenAI()

# After
client = OpenAI(base_url="http://localhost:8080/v1", api_key="gw_live_...")
```

The rest of your code is unchanged.

---

## Python

```python
from openai import OpenAI

client = OpenAI(
    base_url="http://localhost:8080/v1",
    api_key="gw_live_...",  # generate: gateway keys create --name dev
)

response = client.chat.completions.create(
    model="standard",  # tier name, not a model ID
    messages=[{"role": "user", "content": "Hello"}],
)
print(response.choices[0].message.content)
```

For local dev without a key: `GATEWAY_AUTH_DISABLED=1 gateway serve`

---

## TypeScript / Node

```typescript
import OpenAI from "openai";

const client = new OpenAI({
  baseURL: "http://localhost:8080/v1",
  apiKey: "gw_live_...",
});

const res = await client.chat.completions.create({
  model: "standard",
  messages: [{ role: "user", content: "Hello" }],
});
console.log(res.choices[0].message.content);
```

---

## Rust

Using [`reqwest`](https://crates.io/crates/reqwest) (or `async-openai` with `with_api_base`):

```rust
let response = reqwest::Client::new()
    .post("http://localhost:8080/v1/chat/completions")
    .bearer_auth("gw_live_...")
    .json(&serde_json::json!({
        "model": "standard",
        "messages": [{"role": "user", "content": "Hello"}]
    }))
    .send().await?;
```

See [`examples/rust/`](../examples/rust/) for a full working example with error handling.

---

## Model names

| OpenAI model  | AccelMars tier | Notes                               |
|---------------|----------------|-------------------------------------|
| `gpt-4o-mini` | `quick`        | Fastest, cheapest (~$0/M free tier) |
| `gpt-4o`      | `standard`     | Default quality tier (~$0.28/M)     |
| `o1`, `o3`    | `max`          | Reasoning-capable, quality-critical |
| best-in-class | `ultra`        | Absolute best; use sparingly        |

---

## What works identically

- Chat completions ✅
- Streaming ✅
- Tool / function calls ✅
- System messages ✅
- `max_tokens`, `temperature` ✅

---

## What's different

- **Model names** — use tier names (`quick`, `standard`, `max`, `ultra`), not model IDs
- **API key** — a gateway key (`gw_live_...`); the gateway authenticates to providers for you
- **No org/project headers** — gateway handles all provider auth
- **`base_url` required** — there is no default; you must point the SDK at your gateway

---

## Next steps

- [TESTING.md](TESTING.md) — test with zero API keys using mock mode (`GATEWAY_MODE=mock`)
- [examples/python/](../examples/python/) — copy-paste Python examples
- [examples/ts/](../examples/ts/) — copy-paste TypeScript examples
- [CLIENT-INTEGRATION.md](CLIENT-INTEGRATION.md) — full API reference and routing constraints

---

_AccelMars Co., Ltd. — gateway · v0.3.x_
