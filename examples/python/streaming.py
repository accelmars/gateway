"""Streaming chat completion via AccelMars Gateway.

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

stream = client.chat.completions.create(
    model="standard",
    messages=[{"role": "user", "content": "Count to 5, one number per line."}],
    stream=True,
)

for chunk in stream:
    delta = chunk.choices[0].delta.content
    if delta is not None:
        print(delta, end="", flush=True)

print()
