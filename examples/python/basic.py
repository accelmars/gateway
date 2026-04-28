"""Basic chat completion via AccelMars Gateway.

Prerequisites:
    pip install openai
    gateway serve  (or GATEWAY_MODE=mock gateway serve)
"""
import os
from openai import OpenAI

GATEWAY_URL = os.environ.get("ACCELMARS_GATEWAY_URL", "http://localhost:4000")

client = OpenAI(
    base_url=f"{GATEWAY_URL}/v1",
    api_key="local",
)

response = client.chat.completions.create(
    model="standard",
    messages=[
        {"role": "system", "content": "You are a helpful assistant."},
        {"role": "user", "content": "What is 2 + 2?"},
    ],
)

print(response.choices[0].message.content)
print(f"\nModel: {response.model}  |  Tokens: {response.usage.total_tokens}")
