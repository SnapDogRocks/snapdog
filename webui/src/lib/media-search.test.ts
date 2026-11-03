import { afterEach, describe, expect, it, vi } from "vitest";
import { api } from "./api";

// The search and the two Subsonic play calls of the media API (MED-0011 M2): what goes on the wire.

interface Call {
  url: string;
  method: string | undefined;
}

function stubFetch(body: string): Call[] {
  const calls: Call[] = [];
  vi.stubGlobal(
    "fetch",
    vi.fn((url: string, init?: RequestInit) => {
      calls.push({ url, method: init?.method });
      return Promise.resolve(new Response(body, { status: 200 }));
    }),
  );
  return calls;
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("media search", () => {
  it("encodes the text and sends only the options that were given", async () => {
    const calls = stubFetch('{"items":[],"next_offset":null}');
    await api.media.search("a&b=c é");
    expect(calls[0]?.url).toBe("/api/v1/media/search?q=a%26b%3Dc+%C3%A9");
  });

  it("joins the kinds and passes limit and offset", async () => {
    const calls = stubFetch('{"items":[]}');
    await api.media.search("elton", { kinds: ["album", "track"], limit: 8, offset: 0 });
    expect(calls[0]?.url).toBe("/api/v1/media/search?q=elton&kind=album%2Ctrack&limit=8&offset=0");
  });

  it("returns the items of the response", async () => {
    stubFetch('{"items":[{"kind":"album","title":"Greatest","duration":9,"album_id":"al-1"}],"next_offset":8}');
    const res = await api.media.search("gr");
    expect(res.items[0]?.album_id).toBe("al-1");
    expect(res.next_offset).toBe(8);
  });
});

describe("play from a search result", () => {
  it("plays an album under its own endpoint, with the id encoded", async () => {
    const calls = stubFetch("");
    await api.zones.playSubsonicAlbum(2, "al-1/x y");
    expect(calls[0]).toEqual({ url: "/api/v1/zones/2/play/subsonic/album/al-1%2Fx%20y", method: "POST" });
  });

  it("plays a track by its track id", async () => {
    const calls = stubFetch("");
    await api.zones.playSubsonicTrack(2, "tr/1");
    expect(calls[0]).toEqual({ url: "/api/v1/zones/2/play/subsonic/tr%2F1", method: "POST" });
  });
});
