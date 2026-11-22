import { readFileSync } from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";
import { WS_EVENT_TYPES } from "./ws-events.generated";

describe("generated WebSocket event types", () => {
  it("match the events in the server's AsyncAPI document", () => {
    // A new server event regenerates docs/api/asyncapi.json; this fails until
    // `pnpm run ws:generate` runs, and then the type check fails until the event is handled.
    const doc = JSON.parse(
      readFileSync(path.resolve(__dirname, "../../../docs/api/asyncapi.json"), "utf8"),
    ) as { components: { messages: Record<string, { name: string }> } };
    const server = Object.values(doc.components.messages).map((m) => m.name).sort();
    expect([...WS_EVENT_TYPES]).toEqual(server);
  });
});
