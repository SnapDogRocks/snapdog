import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ClientInfo, WsZoneChanged, ZoneInfo } from "@/lib/types";
import { useAppStore } from "@/stores/useAppStore";
import { applyNotification } from "./useAppSession";

vi.mock("@/lib/api", () => ({ api: {} }));

const ZONE: ZoneInfo = {
  index: 1,
  name: "Kitchen",
  icon: "",
  volume: 40,
  muted: false,
  playback: "playing",
  source: "radio",
  shuffle: false,
  repeat: "off",
  presence: false,
  presence_enabled: true,
  presence_timer_active: false,
};

const CLIENT: ClientInfo = {
  index: 2,
  name: "Old name",
  mac: "00:00:00:00:00:02",
  zone_index: 1,
  icon: "",
  volume: 50,
  max_volume: 100,
  muted: false,
  connected: true,
  is_snapdog: true,
};

function zoneChanged(patch: Partial<WsZoneChanged>): WsZoneChanged {
  return {
    type: "zone_changed",
    zone: 1,
    playback: "playing",
    source: "url",
    shuffle: false,
    repeat: "off",
    title: "T",
    artist: "A",
    album: "",
    album_artist: null,
    genre: null,
    year: null,
    track_number: null,
    disc_number: null,
    duration_ms: 0,
    position_ms: 0,
    seekable: false,
    cover_url: null,
    bitrate_kbps: null,
    content_type: null,
    track_index: null,
    track_count: null,
    playlist_index: null,
    playlist_id: null,
    playlist_name: null,
    playlist_total: null,
    can_playlist_next: false,
    can_playlist_prev: false,
    can_next: false,
    can_prev: false,
    volume: 40,
    muted: false,
    ...patch,
  };
}

describe("applyNotification", () => {
  beforeEach(() => {
    useAppStore.setState({
      zones: new Map([[1, { ...ZONE, track: null, presenceEnabled: true, presenceTimerActive: false, buffered_ms: null, error: null }]]),
      clients: new Map([[2, CLIENT]]),
    });
  });

  it("renames a client and keeps the name when an older server sends none", () => {
    const base = { type: "client_state_changed", client: 2, volume: 50, muted: false, connected: true, zone: 1, is_snapdog: true } as const;
    applyNotification({ ...base, name: "New name", latency_ms: 20 });
    expect(useAppStore.getState().clients.get(2)).toMatchObject({ name: "New name", latency_ms: 20 });

    applyNotification(base);
    expect(useAppStore.getState().clients.get(2)).toMatchObject({ name: "New name", latency_ms: 20 });
  });

  it("replaces the sample rate and source of the previous track", () => {
    applyNotification(zoneChanged({ sample_rate: 48000, source: "url" }));
    expect(useAppStore.getState().zones.get(1)?.track).toMatchObject({ sample_rate: 48000, source: "url" });

    applyNotification(zoneChanged({ sample_rate: 44100, source: "radio" }));
    expect(useAppStore.getState().zones.get(1)?.track).toMatchObject({ sample_rate: 44100, source: "radio" });

    applyNotification(zoneChanged({}));
    expect(useAppStore.getState().zones.get(1)?.track?.sample_rate).toBeNull();
  });

  it("updates the client EQ state from client_eq_changed", () => {
    applyNotification({ type: "client_eq_changed", client: 2, enabled: true, bands: [], preset: null });
    expect(useAppStore.getState().clients.get(2)?.eq_enabled).toBe(true);
    applyNotification({ type: "client_eq_changed", client: 2, enabled: false, bands: [] });
    expect(useAppStore.getState().clients.get(2)?.eq_enabled).toBe(false);
  });

  it("ignores an event type it does not know", () => {
    expect(() => { applyNotification({ type: "from_a_newer_server" } as never); }).not.toThrow();
  });
});
