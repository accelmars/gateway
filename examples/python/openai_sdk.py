"""OpenAI SDK drop-in replacement via AccelMars Gateway.

Two-line change from your existing OpenAI code:
    1. Add base_url pointing at the gateway
    2. Set api_key to "local" (gateway manages provider keys server-side)

Prerequisites:
    pip install openai
    gateway serve  (or GATEWAY_MODE=mock gateway serve)
"""
import os

# Before (OpenAI direct):
# from openai import OpenAI
# client = OpenAI()  # reads OPENAI_API_KEY, calls api.openai.com

# After (AccelMars Gateway) — only two lines changed:
from openai import OpenAI

GATEWAY_URL = os.environ.get("ACCELMARS_GATEWAY_URL", "http://localhost:4000")

client = OpenAI(
    base_url=f"{GATEWAY_URL}/v1",  # <- changed: point at gateway
    api_key="local",               # <- changed: gateway manages provider keys
)

# Everything else stays the same — use tier name instead of model ID
response = client.chat.completions.create(
    model="standard",  # "standard" tier, not "gpt-4" or "claude-sonnet-4-6"
    messages=[{"role": "user", "content": "Explain gradient descent in one sentence."}],
)

print(response.choices[0].message.content)
print(f"\nActual model used: {response.model}")
