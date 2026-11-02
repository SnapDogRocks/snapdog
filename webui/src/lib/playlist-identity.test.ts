import { readdirSync, readFileSync, statSync } from "node:fs";
import path from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { api } from "./api";
import type { PlaylistInfo, WsZoneChanged } from "./types";
import { trackFromZoneChanged } from "./ws-zone";

// Exit criterion of the media identity contract (MED-0011): the web UI neither
// sends nor reads the deprecated playlist `id`, nor `zone_changed.playlist`.

interface Call {
  url: string;
  method: string | undefined;
  body: unknown;
}

function stubFetch(): Call[] {
  const calls: Call[] = [];
  vi.stubGlobal(
    "fetch",
    vi.fn((url: string, init?: RequestInit) => {
      calls.push({
        url,
        method: init?.method,
        body: typeof init?.body === "string" ? (JSON.parse(init.body) as unknown) : undefined,
      });
      return Promise.resolve(new Response("[]", { status: 200 }));
    }),
  );
  return calls;
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("playlist calls", () => {
  it("sends { index, track } to the index endpoint and no id key", async () => {
    const calls = stubFetch();
    await api.zones.playPlaylist(2, 5, 3);
    expect(calls).toHaveLength(1);
    expect(calls[0]?.url).toBe("/api/v1/zones/2/play/playlist");
    expect(calls[0]?.body).toStrictEqual({ index: 5, track: 3 });
    expect(Object.keys(calls[0]?.body as object)).not.toContain("id");
  });

  it("sends { playlist_id, track } to the by-id endpoint and no id key", async () => {
    const calls = stubFetch();
    await api.zones.playPlaylistById(1, "subsonic:42", 0);
    expect(calls[0]?.url).toBe("/api/v1/zones/1/play/playlist/by-id");
    expect(calls[0]?.body).toStrictEqual({ playlist_id: "subsonic:42", track: 0 });
    expect(Object.keys(calls[0]?.body as object)).not.toContain("id");
  });

  it("percent-encodes the playlist_id in by-id paths", async () => {
    const calls = stubFetch();
    await api.media.tracksById("subsonic:a/b");
    await api.media.playlistById("snapdog:radio");
    expect(calls[0]?.url).toBe("/api/v1/media/playlists/by-id/subsonic%3Aa%2Fb/tracks");
    expect(calls[1]?.url).toBe("/api/v1/media/playlists/by-id/snapdog%3Aradio");
  });
});

describe("zone_changed decoding", () => {
  it("takes the playlist identity from playlist_index and playlist_id and ignores playlist", () => {
    // Payload of a server that no longer sends the deprecated `playlist`.
    const payload = {
      type: "zone_changed",
      zone: 1,
      playback: "playing",
      source: "subsonic_playlist",
      shuffle: false,
      repeat: "off",
      title: "t",
      artist: "a",
      album: "b",
      album_artist: null,
      genre: null,
      year: null,
      track_number: 1,
      disc_number: null,
      duration_ms: 1000,
      position_ms: 0,
      seekable: true,
      cover_url: null,
      bitrate_kbps: null,
      content_type: null,
      track_index: 2,
      track_count: 9,
      playlist_index: 4,
      playlist_id: "subsonic:42",
      playlist_name: "Mix",
      playlist_total: 9,
      can_playlist_next: true,
      can_playlist_prev: true,
      can_next: true,
      can_prev: true,
      volume: 50,
      muted: false,
    } satisfies WsZoneChanged;
    expect("playlist" in payload).toBe(false);
    const track = trackFromZoneChanged(payload);
    expect(track.playlist_index).toBe(4);
    expect(track.playlist_id).toBe("subsonic:42");
    expect(track.playlist_track_index).toBe(2);
  });

  it("does not take the identity from a stale playlist field", () => {
    const withLegacy = { playlist: 99, playlist_index: 1, playlist_id: "snapdog:radio" } as unknown as WsZoneChanged;
    const track = trackFromZoneChanged(withLegacy);
    expect(track.playlist_index).toBe(1);
    expect(track.playlist_id).toBe("snapdog:radio");
  });
});

describe("typed client", () => {
  it("has no id on PlaylistInfo and no playlist on WsZoneChanged", () => {
    const info: PlaylistInfo = {
      index: 0,
      playlist_id: "snapdog:radio",
      name: "Radio",
      song_count: 0,
      duration: 0,
      cover_art: null,
    };
    // @ts-expect-error `id` is deprecated and not part of the client type
    expect(info.id).toBeUndefined();
    const keys: (keyof WsZoneChanged)[] = ["playlist_index", "playlist_id"];
    // @ts-expect-error `playlist` is deprecated and not part of the client type
    keys.push("playlist");
    expect(keys).toHaveLength(3);
  });
});

function sourceFiles(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const full = path.join(dir, name);
    if (statSync(full).isDirectory()) return sourceFiles(full);
    return /\.(ts|tsx)$/.test(name) && !/\.test\.tsx?$/.test(name) ? [full] : [];
  });
}

describe("source guard", () => {
  const root = path.resolve(import.meta.dirname, "..");
  const files = sourceFiles(root).map((f) => ({ f: path.relative(root, f), text: readFileSync(f, "utf8") }));

  it("never sends { id: ... } in a playlist call", () => {
    const offenders = files.filter(({ text }) => /play\/playlist[^;]*\{\s*id\s*:/.test(text)).map(({ f }) => f);
    expect(offenders).toEqual([]);
  });

  it("never reads the deprecated zone_changed.playlist field", () => {
    const offenders = files.filter(({ text }) => /\bn\.playlist\b(?![\w(])/.test(text)).map(({ f }) => f);
    expect(offenders).toEqual([]);
  });

  it("never reads the id of a playlist list entry", () => {
    const offenders = files
      .filter(({ text }) => /\b(pl|p|playlist)\.id\b/.test(text))
      .map(({ f }) => f);
    expect(offenders).toEqual([]);
  });
});
