import { create } from "zustand";
import type {
  ZoneInfo,
  TrackMetadata,
  ClientInfo,
  EqBand,
} from "@/lib/types";
import { api } from "@/lib/api";

const DEFAULT_TRACK: TrackMetadata = {
  title: "",
  artist: "",
  album: "",
  album_artist: null,
  genre: null,
  year: null,
  track_number: null,
  disc_number: null,
  duration_ms: 0,
  position_ms: 0,
  seekable: false,
  bitrate_kbps: null,
  content_type: null,
  sample_rate: null,
  source: "idle",
  cover_url: null,
  playlist_index: null,
  playlist_id: null,
  playlist_name: null,
  playlist_total: null,
  playlist_track_index: null,
  playlist_track_count: null,
  can_playlist_next: false,
  can_playlist_prev: false,
  can_next: false,
  can_prev: false,
};

// ── Zone with track metadata merged ───────────────────────────

export interface ZoneState extends ZoneInfo {
  track: TrackMetadata | null;
  presenceEnabled: boolean;
  presenceTimerActive: boolean;
  buffered_ms: number | null;
  eqEnabled?: boolean;
  error: PlaybackErrorState | null;
}

export interface PlaybackErrorState {
  message: string;
  details: string | null;
  recoverable: boolean;
}

type ZoneMutation = (zone: ZoneState | undefined) => ZoneState | undefined;
type ClientMutation = (client: ClientInfo | undefined) => ClientInfo | undefined;

interface LoadRequest {
  id: number;
  authEpoch: number;
  zoneMutations: { id: number; apply: ZoneMutation }[];
  clientMutations: { id: number; apply: ClientMutation }[];
}

const activeLoadRequests = new Set<LoadRequest>();
let authEpoch = 0;
let authListenerWindow: Window | null = null;
let nextLoadId = 0;
let activeLoadId = 0;
let pendingLoad: { id: number; authEpoch: number; promise: Promise<void> } | null = null;
let queuedLoad: Promise<void> | null = null;

function ensureAuthChangeListener() {
  if (typeof window === "undefined" || authListenerWindow === window) return;
  authListenerWindow?.removeEventListener("snapdog:auth-changed", handleAuthChanged);
  authListenerWindow = window;
  authListenerWindow.addEventListener("snapdog:auth-changed", handleAuthChanged);
}

function handleAuthChanged() {
  authEpoch++;
}

function recordZoneMutation(id: number, apply: ZoneMutation) {
  for (const request of activeLoadRequests) request.zoneMutations.push({ id, apply });
}

function recordClientMutation(id: number, apply: ClientMutation) {
  for (const request of activeLoadRequests) request.clientMutations.push({ id, apply });
}

function zoneFromInfo(z: ZoneInfo, existing?: ZoneState): ZoneState {
  return {
    ...z,
    track: existing?.track ?? null,
    presenceEnabled: existing?.presenceEnabled ?? true,
    presenceTimerActive: existing?.presenceTimerActive ?? false,
    buffered_ms: existing?.buffered_ms ?? null,
    eqEnabled: existing?.eqEnabled ?? false,
    error: existing?.error ?? null,
  };
}

// ── Store shape ───────────────────────────────────────────────

interface AppState {
  zones: Map<number, ZoneState>;
  clients: Map<number, ClientInfo>;
  selectedZone: number;
  isConnected: boolean;
  serverGoingAway: boolean;
  isLoading: boolean;
  isRefreshing: boolean;
  hasLoaded: boolean;
  loadError: string | null;
  needsAuth: boolean;
  /** Bumped when the playlist list must be reloaded (`playlists_changed`, reconnect). */
  playlistsVersion: number;
  bumpPlaylists: () => void;

  // Init
  /**
   * Load zones and clients. A call during a pending load joins it; with `fresh`, it
   * instead loads once more after the pending one, so that a state read before the
   * caller's point in time (e.g. before the WebSocket subscribed) is not the last word.
   */
  loadAll: (options?: { fresh?: boolean }) => Promise<void>;

  // Zone updates
  setZones: (zones: ZoneInfo[]) => void;
  updateZone: (
    id: number,
    patch: Partial<Pick<ZoneState, "playback" | "volume" | "muted" | "source" | "shuffle" | "repeat"> & { track: Partial<TrackMetadata> | null }>,
  ) => void;
  updateZoneVolume: (id: number, volume: number, muted: boolean) => void;
  updateZoneProgress: (id: number, position_ms: number, duration_ms: number, buffered_ms: number | null) => void;
  updateZonePresence: (id: number, presence: boolean, enabled: boolean, timerActive: boolean) => void;
  updateZoneEq: (id: number, enabled: boolean, bands?: EqBand[], preset?: string) => void;
  setZoneError: (id: number, error: PlaybackErrorState | null) => void;

  // Client updates
  setClients: (clients: ClientInfo[]) => void;
  updateClient: (
    id: number,
    patch: Partial<Pick<ClientInfo, "name" | "volume" | "muted" | "connected" | "zone_index" | "is_snapdog" | "latency_ms" | "eq_enabled">>,
  ) => void;

  // UI
  selectZone: (id: number) => void;
  setConnected: (v: boolean, serverGoingAway?: boolean) => void;
}

export const useAppStore = create<AppState>((set, get) => ({
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

  bumpPlaylists: () => { set((s) => ({ playlistsVersion: s.playlistsVersion + 1 })); },

  loadAll: (options) => {
    ensureAuthChangeListener();
    const requestAuthEpoch = authEpoch;
    if (pendingLoad?.authEpoch === requestAuthEpoch) {
      if (!options?.fresh) return pendingLoad.promise;
      queuedLoad ??= pendingLoad.promise.then(() => {
        queuedLoad = null;
        return get().loadAll();
      });
      return queuedLoad;
    }

    const request: LoadRequest = {
      id: ++nextLoadId,
      authEpoch: requestAuthEpoch,
      zoneMutations: [],
      clientMutations: [],
    };
    activeLoadId = request.id;
    activeLoadRequests.add(request);
    const hasLoaded = get().hasLoaded;
    set({ isLoading: !hasLoaded, isRefreshing: hasLoaded, loadError: null });

    const isCurrent = () =>
      authEpoch === request.authEpoch && activeLoadId === request.id;

    const promise = (async () => {
      try {
        const [zoneList, clientList] = await Promise.all([
          api.zones.list(),
          api.clients.list(),
        ]);

        // Fetch track metadata and EQ state for each zone in parallel.
        const [metaResults, eqResults] = await Promise.all([
          Promise.allSettled(zoneList.map((z) => api.zones.getTrackMetadata(z.index))),
          Promise.allSettled(zoneList.map((z) => api.eq.get(z.index))),
        ]);

        if (!isCurrent()) return;

        const zones = new Map<number, ZoneState>();
        for (const [i, z] of zoneList.entries()) {
          const metaResult = metaResults[i];
          const eqResult = eqResults[i];
          zones.set(z.index, {
            ...z,
            track: metaResult?.status === "fulfilled" ? metaResult.value : null,
            presenceEnabled: z.presence_enabled,
            presenceTimerActive: z.presence_timer_active,
            buffered_ms: null,
            eqEnabled: eqResult?.status === "fulfilled" ? eqResult.value.enabled : false,
            error: null,
          });
        }

        // Replay WebSocket/store updates received after this snapshot began.
        for (const mutation of request.zoneMutations) {
          const updated = mutation.apply(zones.get(mutation.id));
          if (updated) zones.set(mutation.id, updated);
          else zones.delete(mutation.id);
        }

        const clients = new Map<number, ClientInfo>();
        for (const c of clientList) clients.set(c.index, c);
        for (const mutation of request.clientMutations) {
          const updated = mutation.apply(clients.get(mutation.id));
          if (updated) clients.set(mutation.id, updated);
          else clients.delete(mutation.id);
        }

        const current = get();
        const stored = typeof window !== "undefined" ? Number(sessionStorage.getItem("selectedZone")) : 0;
        const initial = current.hasLoaded && current.selectedZone && zones.has(current.selectedZone)
          ? current.selectedZone
          : stored && zones.has(stored)
            ? stored
            : (zoneList[0]?.index ?? 1);
        set({
          zones,
          clients,
          selectedZone: initial,
          isLoading: false,
          isRefreshing: false,
          hasLoaded: true,
          loadError: null,
          needsAuth: false,
        });
      } catch (e) {
        if (!isCurrent()) return;
        const status = e instanceof Error && "status" in e ? (e as { status: number }).status : 0;
        const message = e instanceof Error ? e.message : String(e);
        set({
          isLoading: false,
          isRefreshing: false,
          loadError: message,
          needsAuth: status === 401,
        });
      } finally {
        activeLoadRequests.delete(request);
        if (pendingLoad?.id === request.id) pendingLoad = null;
        if (authEpoch !== request.authEpoch && activeLoadRequests.size === 0 && !get().hasLoaded) {
          set({ isLoading: false, isRefreshing: false });
        }
      }
    })();
    pendingLoad = { id: request.id, authEpoch: request.authEpoch, promise };
    return promise;
  },

  setZones: (list) => {
    const existing = get().zones;
    const byId = new Map(list.map((zone) => [zone.index, zone]));
    const ids = new Set([...existing.keys(), ...byId.keys()]);
    const zones = new Map<number, ZoneState>();
    for (const id of ids) {
      const incoming = byId.get(id);
      const apply: ZoneMutation = (current) => incoming ? zoneFromInfo(incoming, current) : undefined;
      recordZoneMutation(id, apply);
      const updated = apply(existing.get(id));
      if (updated) zones.set(id, updated);
    }
    set({ zones });
  },

  updateZone: (id, patch) => {
    const zones = new Map(get().zones);
    const apply: ZoneMutation = (z) => {
      if (!z) return undefined;
      const { track: trackPatch, ...rest } = patch;
      const updatedTrack = trackPatch === null ? null : trackPatch ? { ...DEFAULT_TRACK, ...z.track, ...trackPatch } : z.track;
      const clearsError = rest.playback === "playing" || (trackPatch !== undefined && trackPatch !== null);
      return { ...z, ...rest, track: updatedTrack, error: clearsError ? null : z.error };
    };
    recordZoneMutation(id, apply);
    const updated = apply(zones.get(id));
    if (updated) zones.set(id, updated);
    set({ zones });
  },

  updateZoneVolume: (id, volume, muted) => {
    const zones = new Map(get().zones);
    const apply: ZoneMutation = (z) => z ? { ...z, volume, muted } : undefined;
    recordZoneMutation(id, apply);
    const updated = apply(zones.get(id));
    if (updated) zones.set(id, updated);
    set({ zones });
  },

  updateZoneProgress: (id, position_ms, duration_ms, buffered_ms) => {
    const zones = new Map(get().zones);
    const apply: ZoneMutation = (z) =>
      z?.track ? { ...z, track: { ...z.track, position_ms, duration_ms }, buffered_ms } : z;
    recordZoneMutation(id, apply);
    const updated = apply(zones.get(id));
    if (updated) zones.set(id, updated);
    set({ zones });
  },

  updateZonePresence: (id, presence, enabled, timerActive) => {
    const zones = new Map(get().zones);
    const apply: ZoneMutation = (z) => z
      ? { ...z, presence, presenceEnabled: enabled, presenceTimerActive: timerActive }
      : undefined;
    recordZoneMutation(id, apply);
    const updated = apply(zones.get(id));
    if (updated) zones.set(id, updated);
    set({ zones });
  },

  updateZoneEq: (id, enabled) => {
    const zones = new Map(get().zones);
    const apply: ZoneMutation = (z) => z ? { ...z, eqEnabled: enabled } : undefined;
    recordZoneMutation(id, apply);
    const updated = apply(zones.get(id));
    if (updated) zones.set(id, updated);
    set({ zones });
  },

  setZoneError: (id, error) => {
    const zones = new Map(get().zones);
    const apply: ZoneMutation = (z) => z ? { ...z, error } : undefined;
    recordZoneMutation(id, apply);
    const updated = apply(zones.get(id));
    if (updated) zones.set(id, updated);
    set({ zones });
  },

  setClients: (list) => {
    const existing = get().clients;
    const byId = new Map(list.map((client) => [client.index, client]));
    const ids = new Set([...existing.keys(), ...byId.keys()]);
    const clients = new Map<number, ClientInfo>();
    for (const id of ids) {
      const incoming = byId.get(id);
      const apply: ClientMutation = (current) => incoming ? { ...current, ...incoming } : undefined;
      recordClientMutation(id, apply);
      const updated = apply(existing.get(id));
      if (updated) clients.set(id, updated);
    }
    set({ clients });
  },

  updateClient: (id, patch) => {
    const clients = new Map(get().clients);
    const apply: ClientMutation = (client) => client ? { ...client, ...patch } : undefined;
    recordClientMutation(id, apply);
    const updated = apply(clients.get(id));
    if (updated) clients.set(id, updated);
    set({ clients });
  },

  selectZone: (id) => {
    sessionStorage.setItem("selectedZone", String(id));
    set({ selectedZone: id });
  },
  setConnected: (v, goingAway) => {
    const was = get().isConnected;
    set({ isConnected: v, serverGoingAway: goingAway ?? false });
    // On (re)connect, re-fetch all state. Fresh: a load already running read its state
    // before the socket subscribed, and a change in between would be missed.
    if (v && !was) {
      get().bumpPlaylists();
      void get().loadAll({ fresh: true });
    }
  },
}));
