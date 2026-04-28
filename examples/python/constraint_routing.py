"""Routing constraints via AccelMars Gateway.

Demonstrates the sensitive_excluded policy: when privacy=sensitive,
providers with data-residency concerns (e.g., DeepSeek) are excluded.

Gateway config reference (gateway.toml):
    [constraints]
    sensitive_excluded = ["deepseek"]

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

PROMPT = "Summarize the privacy policy implications of large-scale data collection."

# Default routing — any provider eligible, including DeepSeek
print("=== privacy: open (default) ===")
response = client.chat.completions.create(
    model="standard",
    messages=[{"role": "user", "content": PROMPT}],
    extra_body={"metadata": {"privacy": "open"}},
)
print(f"Provider selected: {response.model}")
print(f"Response: {response.choices[0].message.content}\n")

# Sensitive routing — sensitive_excluded providers (DeepSeek) are skipped
print("=== privacy: sensitive (sensitive_excluded policy) ===")
print("Excluded by config: deepseek")
response = client.chat.completions.create(
    model="standard",
    messages=[{"role": "user", "content": PROMPT}],
    extra_body={"metadata": {"privacy": "sensitive"}},
)
print(f"Provider selected: {response.model}")
print(f"Response: {response.choices[0].message.content}")
