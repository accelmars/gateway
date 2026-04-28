/**
 * Basic chat completion via AccelMars Gateway.
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

const response = await client.chat.completions.create({
  model: "standard",
  messages: [
    { role: "system", content: "You are a helpful assistant." },
    { role: "user", content: "What is 2 + 2?" },
  ],
});

console.log(response.choices[0].message.content);
console.log(`\nModel: ${response.model}  |  Tokens: ${response.usage?.total_tokens}`);
