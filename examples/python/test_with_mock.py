"""pytest tests using AccelMars Gateway in mock mode.

Mock mode returns deterministic responses — zero API keys, zero cost.

Start the gateway in mock mode before running:
    GATEWAY_MODE=mock gateway serve

Then run:
    pytest examples/python/test_with_mock.py -v

Prerequisites:
    pip install openai pytest
"""
import os
import pytest
from openai import OpenAI

GATEWAY_URL = os.environ.get("ACCELMARS_GATEWAY_URL", "http://localhost:4000")


@pytest.fixture
def client():
    return OpenAI(
        base_url=f"{GATEWAY_URL}/v1",
        api_key="local",
    )


def test_basic_completion(client):
    response = client.chat.completions.create(
        model="standard",
        messages=[{"role": "user", "content": "Hello"}],
    )
    assert response.choices[0].message.role == "assistant"
    assert response.choices[0].message.content is not None
    assert response.choices[0].finish_reason == "stop"
    assert response.usage.total_tokens > 0


def test_streaming_completion(client):
    chunks = []
    stream = client.chat.completions.create(
        model="standard",
        messages=[{"role": "user", "content": "Say hello"}],
        stream=True,
    )
    for chunk in stream:
        content = chunk.choices[0].delta.content
        if content:
            chunks.append(content)
    assert len(chunks) > 0


def test_all_tiers(client):
    for tier in ["quick", "standard", "max"]:
        response = client.chat.completions.create(
            model=tier,
            messages=[{"role": "user", "content": "Ping"}],
        )
        assert response.choices[0].message.content is not None


if __name__ == "__main__":
    pytest.main([__file__, "-v"])
