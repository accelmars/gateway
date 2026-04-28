"""Explicit tier selection via AccelMars Gateway.

Sends the same prompt to quick, standard, and max tiers and compares
model, latency, and response quality.

Tier cost reference (approximate):
    quick    ~$0.00/M   high-volume bulk work
    standard ~$0.10/M   production quality
    max      ~$3.00/M   quality-critical reasoning

Prerequisites:
    pip install openai
    gateway serve  (or GATEWAY_MODE=mock gateway serve)
"""
import os
import time
from openai import OpenAI

GATEWAY_URL = os.environ.get("ACCELMARS_GATEWAY_URL", "http://localhost:4000")

client = OpenAI(
    base_url=f"{GATEWAY_URL}/v1",
    api_key="local",
)

PROMPT = "Summarize the theory of relativity in one sentence."
TIERS = ["quick", "standard", "max"]

print(f"Prompt: {PROMPT}\n")
print(f"{'TIER':<10} {'MODEL':<25} {'LATENCY':>8}  RESPONSE")
print("-" * 90)

for tier in TIERS:
    start = time.monotonic()
    response = client.chat.completions.create(
        model=tier,
        messages=[{"role": "user", "content": PROMPT}],
    )
    elapsed = time.monotonic() - start

    content = response.choices[0].message.content or ""
    preview = content[:55].replace("\n", " ")
    if len(content) > 55:
        preview += "..."

    print(f"{tier:<10} {response.model:<25} {elapsed:>6.2f}s  {preview}")
