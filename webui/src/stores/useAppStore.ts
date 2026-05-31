import { create } from "zustand";
import type {
  ZoneInfo,
  TrackMetadata,
  ClientInfo,
} from "@/lib/types";
import { api, type EqConfig } from "@/lib/api";

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
  playlist_track_index: null,
  playlist_track_count: null,
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
}

// ── Store shape ───────────────────────────────────────────────

interface AppState {
  zones: Map<number, ZoneState>;
  clients: Map<number, ClientInfo>;
  selectedZone: number;
  isConnected: boolean;
  serverGoingAway: boolean;
  isLoading: boolean;
  needsAuth: boolean;

  // Init
  loadAll: () => Promise<void>;

  // Zone updates
  setZones: (zones: ZoneInfo[]) => void;
  updateZone: (
    id: number,
    patch: Partial<Pick<ZoneState, "playback" | "volume" | "muted" | "source" | "shuffle" | "repeat" | "track_repeat">>,
  ) => void;
  updateZoneTrack: (
    id: number,
    track: Pick<TrackMetadata, "title" | "artist" | "album" | "album_artist" | "genre" | "year" | "track_number" | "duration_ms" | "position_ms" | "seekable" | "can_next" | "can_prev" | "cover_url">,
  ) => void;
  updateZoneProgress: (id: number, position_ms: number, duration_ms: number, buffered_ms: number | null) => void;
  updateZonePresence: (id: number, presence: boolean, enabled: boolean, timerActive: boolean) => void;
  updateZoneEq: (id: number, enabled: boolean, bands?: Array<{ filter_type: string; frequency: number; gain: number; q: number }>, preset?: string) => void;

  // Client updates
  setClients: (clients: ClientInfo[]) => void;
  updateClient: (
    id: number,
    patch: Partial<Pick<ClientInfo, "volume" | "muted" | "connected" | "zone_index" | "is_snapdog">>,
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
  needsAuth: false,

  loadAll: async () => {
    set({ isLoading: true });
    try {
      const [zoneList, clientList] = await Promise.all([
        api.zones.list(),
        api.clients.list(),
      ]);

      const zones = new Map<number, ZoneState>();
      for (const z of zoneList) {
        zones.set(z.index, { ...z, track: null, presenceEnabled: z.presence_enabled ?? true, presenceTimerActive: z.presence_timer_active ?? false, buffered_ms: null });
      }

      // Fetch track metadata and EQ state for each zone in parallel
      const [metaResults, eqResults] = await Promise.all([
        Promise.allSettled(zoneList.map((z) => api.zones.getTrackMetadata(z.index))),
        Promise.allSettled(zoneList.map((z) => api.eq.get(z.index))),
      ]);

      for (let i = 0; i < zoneList.length; i++) {
        const zoneId = zoneList[i].index;
        const zone = zones.get(zoneId);
        if (zone) {
          if (metaResults[i].status === "fulfilled") {
            zone.track = (metaResults[i] as PromiseFulfilledResult<TrackMetadata>).value;
          }
          if (eqResults[i].status === "fulfilled") {
            zone.eqEnabled = (eqResults[i] as PromiseFulfilledResult<EqConfig>).value.enabled;
          }
        }
      }

      const clients = new Map<number, ClientInfo>();
      for (const c of clientList) clients.set(c.index, c);

      const stored = typeof window !== "undefined" ? Number(sessionStorage.getItem("selectedZone")) : 0;
      const initial = stored && zones.has(stored) ? stored : (zoneList[0]?.index ?? 1);
      set({ zones, clients, selectedZone: initial, isLoading: false, needsAuth: false });
    } catch (e) {
      const status = e instanceof Error && "status" in e ? (e as { status: number }).status : 0;
      set({ isLoading: false, needsAuth: status === 401 });
    }
  },

  setZones: (list) => {
    const zones = new Map<number, ZoneState>();
    for (const z of list) {
      const existing = get().zones.get(z.index);
      zones.set(z.index, {
        ...z,
        track: existing?.track ?? null,
        presenceEnabled: existing?.presenceEnabled ?? true,
        presenceTimerActive: existing?.presenceTimerActive ?? false,
        buffered_ms: existing?.buffered_ms ?? null,
        eqEnabled: existing?.eqEnabled ?? false,
      });
    }
    set({ zones });
  },

  updateZone: (id, patch) => {
    const zones = new Map(get().zones);
    const z = zones.get(id);
    if (z) zones.set(id, { ...z, ...patch });
    set({ zones });
  },

  updateZoneTrack: (id, track) => {
    const zones = new Map(get().zones);
    const z = zones.get(id);
    if (z) {
      zones.set(id, {
        ...z,
        track: { ...DEFAULT_TRACK, ...z.track, ...track },
      });
    }
    set({ zones });
  },

  updateZoneProgress: (id, position_ms, duration_ms, buffered_ms) => {
    const zones = new Map(get().zones);
    const z = zones.get(id);
    if (z?.track) {
      zones.set(id, { ...z, track: { ...z.track, position_ms, duration_ms }, buffered_ms });
    }
    set({ zones });
  },

  updateZonePresence: (id, presence, enabled, timerActive) => {
    const zones = new Map(get().zones);
    const z = zones.get(id);
    if (z) {
      zones.set(id, { ...z, presence, presenceEnabled: enabled, presenceTimerActive: timerActive });
    }
    set({ zones });
  },

  updateZoneEq: (id, enabled) => {
    const zones = new Map(get().zones);
    const z = zones.get(id);
    if (z) {
      zones.set(id, { ...z, eqEnabled: enabled });
    }
    set({ zones });
  },

  setClients: (list) => {
    const clients = new Map<number, ClientInfo>();
    for (const c of list) clients.set(c.index, c);
    set({ clients });
  },

  updateClient: (id, patch) => {
    const clients = new Map(get().clients);
    const c = clients.get(id);
    if (c) clients.set(id, { ...c, ...patch });
    set({ clients });
  },

  selectZone: (id) => {
    sessionStorage.setItem("selectedZone", String(id));
    set({ selectedZone: id });
  },
  setConnected: (v, goingAway) => {
    const was = get().isConnected;
    set({ isConnected: v, serverGoingAway: goingAway ?? false });
    // On reconnect, re-fetch all state
    if (v && !was) {
      get().loadAll();
    }
  },
}));
