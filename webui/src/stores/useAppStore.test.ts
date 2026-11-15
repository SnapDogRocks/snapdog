import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ClientInfo, TrackMetadata, ZoneInfo } from "@/lib/types";
import { useAppStore } from "./useAppStore";

const apiMocks = vi.hoisted(() => ({
  zonesList: vi.fn(),
  clientsList: vi.fn(),
  getTrackMetadata: vi.fn(),
  getEq: vi.fn(),
}));

vi.mock("@/lib/api", () => ({
  api: {
    zones: { list: apiMocks.zonesList, getTrackMetadata: apiMocks.getTrackMetadata },
    clients: { list: apiMocks.clientsList },
    eq: { get: apiMocks.getEq },
  },
}));

function zone(volume: number, index = 1): ZoneInfo {
  return {
    index,
    name: "Living room",
    icon: "speaker",
    volume,
    muted: false,
    playback: "paused",
    source: "radio",
    shuffle: false,
    repeat: "off",
    presence: false,
    presence_enabled: true,
    presence_timer_active: false,
  };
}

function track(): TrackMetadata {
  return {
    title: "Track",
    artist: "Artist",
    album: "Album",
    album_artist: null,
    genre: null,
    year: null,
    track_number: null,
    disc_number: null,
    duration_ms: 100_000,
    position_ms: 0,
    seekable: true,
    bitrate_kbps: null,
    content_type: null,
    sample_rate: null,
    source: "radio",
    cover_url: null,
    playlist_index: null,
    playlist_id: null,
    playlist_name: null,
    playlist_total: null,
    playlist_track_index: null,
    playlist_track_count: null,
    can_playlist_next: true,
    can_playlist_prev: true,
    can_next: true,
    can_prev: true,
  };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((resolvePromise) => { resolve = resolvePromise; });
  return { promise, resolve };
}

function resetStore() {
  useAppStore.setState({
    zones: new Map(),
    clients: new Map(),
    selectedZone: 1,
    isConnected: false,
    serverGoingAway: false,
    isLoading: true,
    isRefreshing: false,
    hasLoaded: false,
    loadError: null,
    needsAuth: false,
    playlistsVersion: 0,
  });
}

describe("app store loading", () => {
  beforeEach(() => {
    resetStore();
    apiMocks.zonesList.mockReset().mockResolvedValue([]);
    apiMocks.clientsList.mockReset().mockResolvedValue([]);
    apiMocks.getTrackMetadata.mockReset().mockResolvedValue(track());
    apiMocks.getEq.mockReset().mockResolvedValue({ enabled: false });
  });

  afterEach(() => vi.unstubAllGlobals());

  it("coalesces concurrent loadAll calls", async () => {
    const zones = deferred<ZoneInfo[]>();
    const clients = deferred<ClientInfo[]>();
    apiMocks.zonesList.mockReturnValue(zones.promise);
    apiMocks.clientsList.mockReturnValue(clients.promise);

    const first = useAppStore.getState().loadAll();
    const second = useAppStore.getState().loadAll();

    expect(second).toBe(first);
    expect(apiMocks.zonesList).toHaveBeenCalledTimes(1);
    expect(apiMocks.clientsList).toHaveBeenCalledTimes(1);
    zones.resolve([]);
    clients.resolve([]);
    await first;
    expect(useAppStore.getState().hasLoaded).toBe(true);
  });

  it("uses the stored zone on the initial load", async () => {
    vi.stubGlobal("window", new EventTarget());
    vi.stubGlobal("sessionStorage", { getItem: vi.fn(() => "2") });
    apiMocks.zonesList.mockResolvedValueOnce([zone(30), zone(40, 2)]);

    await useAppStore.getState().loadAll();

    expect(useAppStore.getState().selectedZone).toBe(2);
  });

  it("replays a live zone change over a stale refresh snapshot", async () => {
    apiMocks.zonesList.mockResolvedValueOnce([zone(30)]);
    apiMocks.clientsList.mockResolvedValue([]);
    await useAppStore.getState().loadAll();

    const staleZoneList = deferred<ZoneInfo[]>();
    apiMocks.zonesList.mockReturnValueOnce(staleZoneList.promise);
    const refresh = useAppStore.getState().loadAll();
    expect(useAppStore.getState().isRefreshing).toBe(true);
    expect(useAppStore.getState().isLoading).toBe(false);

    useAppStore.getState().updateZoneVolume(1, 73, true);
    staleZoneList.resolve([zone(20)]);
    await refresh;

    expect(useAppStore.getState().zones.get(1)).toMatchObject({ volume: 73, muted: true });
    expect(useAppStore.getState().isRefreshing).toBe(false);
    expect(useAppStore.getState().hasLoaded).toBe(true);
  });

  it("ignores a successful response started under an obsolete auth state", async () => {
    const browser = new EventTarget();
    vi.stubGlobal("window", browser);
    const zones = deferred<ZoneInfo[]>();
    apiMocks.zonesList.mockReturnValue(zones.promise);

    const load = useAppStore.getState().loadAll();
    browser.dispatchEvent(new Event("snapdog:auth-changed"));
    zones.resolve([zone(50)]);
    await load;

    expect(useAppStore.getState().zones.size).toBe(0);
    expect(useAppStore.getState().hasLoaded).toBe(false);
    expect(useAppStore.getState().isLoading).toBe(false);
  });

  it("surfaces the initial load error instead of leaving an empty loading state", async () => {
    apiMocks.zonesList.mockRejectedValueOnce(new Error("connection refused"));

    await useAppStore.getState().loadAll();

    expect(useAppStore.getState()).toMatchObject({
      isLoading: false,
      isRefreshing: false,
      hasLoaded: false,
      loadError: "connection refused",
    });
  });
});
