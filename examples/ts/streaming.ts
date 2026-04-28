/**
 * Streaming chat completion via AccelMars Gateway.
 *
 * Prerequisites:
 *   npm install
 *   gateway serve  (or GATEWAY_MODE=mock gateway serve)
 */
import OpenAI from "openai";

const GATEWAY_URL = process.env.ACCELMARS_GATEWAY_URL ?? "http://localhost:4000";

const client = new OpenAI({
  baseURL: `${GATEWAY_URL}/v1`,
  apiKey: "local",
});

const stream = await client.chat.completions.create({
  model: "standard",
  messages: [{ role: "user", content: "Count to 5, one number per line." }],
  stream: true,
});

for await (const chunk of stream) {
  process.stdout.write(chunk.choices[0]?.delta?.content ?? "");
}

console.log();
