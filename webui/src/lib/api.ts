import type {
  ZoneInfo,
  TrackMetadata,
  ClientInfo,
  PlaylistInfo,
  PlaylistDetails,
  PlaylistState,
  TrackInfo,
  SearchKind,
  SearchResponse,
  SystemStatus,
  VersionInfo,
  UpdateStatus,
  HealthResponse,
  VolumeValue,
  EqBand,
  EqConfig,
} from "./types";

export type { EqBand, EqConfig } from "./types";

import { getApiKey, clearApiKey } from "./auth";

const BASE = "";

export class ApiError extends Error {
  constructor(
    public status: number,
    message: string,
  ) {
    super(message);
  }
}

function authHeaders(): Record<string, string> {
  const key = getApiKey();
  return key ? { Authorization: `Bearer ${key}` } : {};
}

function check401(res: Response) {
  if (res.status === 401) {
    clearApiKey();
    // Trigger re-auth by reloading — the store will detect 401 on loadAll
    window.location.reload();
  }
}

async function get<T>(path: string): Promise<T> {
  const res = await fetch(`${BASE}${path}`, { headers: authHeaders() });
  if (!res.ok) { check401(res); throw new ApiError(res.status, `GET ${path}: ${res.status}`); }
  return (await res.json()) as T;
}

async function putJson<T>(path: string, body: unknown): Promise<T> {
  const res = await fetch(`${BASE}${path}`, {
    method: "PUT",
    headers: { "Content-Type": "application/json", ...authHeaders() },
    body: JSON.stringify(body),
  });
  if (!res.ok) { check401(res); throw new ApiError(res.status, `PUT ${path}: ${res.status}`); }
  const text = await res.text();
  if (!text) throw new ApiError(res.status, `PUT ${path}: expected JSON response but got empty body`);
  return JSON.parse(text) as T;
}

async function putVoid(path: string, body: unknown): Promise<void> {
  const res = await fetch(`${BASE}${path}`, {
    method: "PUT",
    headers: { "Content-Type": "application/json", ...authHeaders() },
    body: JSON.stringify(body),
  });
  if (!res.ok) { check401(res); throw new ApiError(res.status, `PUT ${path}: ${res.status}`); }
}

async function postJson<T>(path: string, body?: unknown): Promise<T> {
  const res = await fetch(`${BASE}${path}`, {
    method: "POST",
    headers: { ...(body !== undefined ? { "Content-Type": "application/json" } : {}), ...authHeaders() },
    ...(body !== undefined ? { body: JSON.stringify(body) } : {}),
  });
  if (!res.ok) { check401(res); throw new ApiError(res.status, `POST ${path}: ${res.status}`); }
  const text = await res.text();
  if (!text) throw new ApiError(res.status, `POST ${path}: expected JSON response but got empty body`);
  return JSON.parse(text) as T;
}

async function postVoid(path: string, body?: unknown): Promise<void> {
  const res = await fetch(`${BASE}${path}`, {
    method: "POST",
    headers: { ...(body !== undefined ? { "Content-Type": "application/json" } : {}), ...authHeaders() },
    ...(body !== undefined ? { body: JSON.stringify(body) } : {}),
  });
  if (!res.ok) { check401(res); throw new ApiError(res.status, `POST ${path}: ${res.status}`); }
}

/** Fetch a binary endpoint with auth and trigger a browser download. */
async function downloadFile(path: string, filename: string): Promise<void> {
  const res = await fetch(`${BASE}${path}`, { headers: authHeaders() });
  if (!res.ok) { check401(res); throw new ApiError(res.status, `GET ${path}: ${res.status}`); }
  const url = URL.createObjectURL(await res.blob());
  const a = document.createElement("a");
  a.href = url;
  a.download = filename;
  document.body.appendChild(a);
  a.click();
  a.remove();
  URL.revokeObjectURL(url);
}

// ── Zones ─────────────────────────────────────────────────────

const Z = "/api/v1/zones";

export const zones = {
  list: () => get<ZoneInfo[]>(Z),
  get: (id: number) => get<ZoneInfo>(`${Z}/${id}`),
  count: () => get<number>(`${Z}/count`),

  // Volume
  getVolume: (id: number) => get<number>(`${Z}/${id}/volume`),
  setVolume: (id: number, v: VolumeValue) => putJson<number>(`${Z}/${id}/volume`, v),

  // Mute
  getMute: (id: number) => get<boolean>(`${Z}/${id}/mute`),
  setMute: (id: number, v: boolean) => putVoid(`${Z}/${id}/mute`, v),
  toggleMute: (id: number) => postVoid(`${Z}/${id}/mute/toggle`),

  // Transport
  play: (id: number) => postVoid(`${Z}/${id}/play`),
  pause: (id: number) => postVoid(`${Z}/${id}/pause`),
  stop: (id: number) => postVoid(`${Z}/${id}/stop`),
  next: (id: number) => postVoid(`${Z}/${id}/next`),
  previous: (id: number) => postVoid(`${Z}/${id}/previous`),

  // Shuffle
  getShuffle: (id: number) => get<boolean>(`${Z}/${id}/shuffle`),
  setShuffle: (id: number, v: boolean) => putVoid(`${Z}/${id}/shuffle`, v),
  toggleShuffle: (id: number) => postVoid(`${Z}/${id}/shuffle/toggle`),

  // Repeat
  getRepeat: (id: number) => get<string>(`${Z}/${id}/repeat`),
  setRepeat: (id: number, v: string) => putVoid(`${Z}/${id}/repeat`, v),
  toggleRepeat: (id: number) => postVoid(`${Z}/${id}/repeat/toggle`),

  // Track info
  getTrackMetadata: (id: number) => get<TrackMetadata>(`${Z}/${id}/track/metadata`),
  getTrackPosition: (id: number) => get<number>(`${Z}/${id}/track/position`),
  seekPosition: (id: number, ms: number) => putVoid(`${Z}/${id}/track/position`, { position_ms: ms }),
  getTrackProgress: (id: number) => get<number>(`${Z}/${id}/track/progress`),
  seekProgress: (id: number, v: number) => putVoid(`${Z}/${id}/track/progress`, v),

  // Play specific content
  playTrack: (id: number, track: number) => postVoid(`${Z}/${id}/play/track`, track),
  playUrl: (id: number, url: string) => postVoid(`${Z}/${id}/play/url`, url),
  playPlaylist: (id: number, playlistIndex: number, track?: number) =>
    postVoid(`${Z}/${id}/play/playlist`, { index: playlistIndex, track: track ?? 0 }),
  /** Play by stable `playlist_id`. Prefer this over the index variant for anything the user selected earlier. */
  playPlaylistById: (id: number, playlistId: string, track?: number) =>
    postVoid(`${Z}/${id}/play/playlist/by-id`, { playlist_id: playlistId, track: track ?? 0 }),
  /** Play a Subsonic track by its `track_id` from a search result. */
  playSubsonicTrack: (id: number, trackId: string) =>
    postVoid(`${Z}/${id}/play/subsonic/${encodeURIComponent(trackId)}`),
  /** Queue the tracks of a Subsonic album and play it. An album is no playlist and has no `playlist_id`. */
  playSubsonicAlbum: (id: number, albumId: string) =>
    postVoid(`${Z}/${id}/play/subsonic/album/${encodeURIComponent(albumId)}`),
  playPlaylistTrack: (zoneId: number, playlistIndex: number, track: number) =>
    postVoid(`${Z}/${zoneId}/play/playlist/${playlistIndex}/track`, track),

  // Playlist navigation
  getPlaylist: (id: number) => get<number>(`${Z}/${id}/playlist`),
  setPlaylist: (id: number, v: number) => putVoid(`${Z}/${id}/playlist`, v),
  nextPlaylist: (id: number) => postVoid(`${Z}/${id}/next/playlist`),
  previousPlaylist: (id: number) => postVoid(`${Z}/${id}/previous/playlist`),
  getPlaylistInfo: (id: number) => get<PlaylistState>(`${Z}/${id}/playlist/info`),

  // Zone info
  getName: (id: number) => get<string>(`${Z}/${id}/name`),
  getIcon: (id: number) => get<string>(`${Z}/${id}/icon`),
  getPlayback: (id: number) => get<string>(`${Z}/${id}/playback`),
  getClients: (id: number) => get<number[]>(`${Z}/${id}/clients`),

};

// ── Clients ───────────────────────────────────────────────────

const C = "/api/v1/clients";

export const clients = {
  list: () => get<ClientInfo[]>(C),
  get: (id: number) => get<ClientInfo>(`${C}/${id}`),
  count: () => get<number>(`${C}/count`),

  getVolume: (id: number) => get<number>(`${C}/${id}/volume`),
  setVolume: (id: number, v: VolumeValue) => putJson<number>(`${C}/${id}/volume`, v),

  getMute: (id: number) => get<boolean>(`${C}/${id}/mute`),
  setMute: (id: number, v: boolean) => putVoid(`${C}/${id}/mute`, v),
  toggleMute: (id: number) => postVoid(`${C}/${id}/mute/toggle`),

  getLatency: (id: number) => get<number>(`${C}/${id}/latency`),
  setLatency: (id: number, v: number) => putVoid(`${C}/${id}/latency`, v),

  getZone: (id: number) => get<number>(`${C}/${id}/zone`),
  setZone: (id: number, zoneId: number) => putVoid(`${C}/${id}/zone`, zoneId),

  getName: (id: number) => get<string>(`${C}/${id}/name`),
  setName: (id: number, name: string) => putVoid(`${C}/${id}/name`, name),

  getIcon: (id: number) => get<string>(`${C}/${id}/icon`),
  getConnected: (id: number) => get<boolean>(`${C}/${id}/connected`),
};

// ── Media ─────────────────────────────────────────────────────

const M = "/api/v1/media";

export const media = {
  playlists: () => get<PlaylistInfo[]>(`${M}/playlists`),
  playlist: (index: number) => get<PlaylistDetails>(`${M}/playlists/${index}`),
  playlistById: (playlistId: string) =>
    get<PlaylistDetails>(`${M}/playlists/by-id/${encodeURIComponent(playlistId)}`),
  tracks: (index: number) => get<TrackInfo[]>(`${M}/playlists/${index}/tracks`),
  tracksById: (playlistId: string) =>
    get<TrackInfo[]>(`${M}/playlists/by-id/${encodeURIComponent(playlistId)}/tracks`),
  track: (index: number, idx: number) => get<TrackInfo>(`${M}/playlists/${index}/tracks/${idx}`),
  /** Search the media server. `limit` and `offset` apply to each kind. Needs at least two characters. */
  search: (q: string, opts: { kinds?: SearchKind[]; limit?: number; offset?: number } = {}) => {
    const params = new URLSearchParams({ q });
    if (opts.kinds) params.set("kind", opts.kinds.join(","));
    if (opts.limit !== undefined) params.set("limit", String(opts.limit));
    if (opts.offset !== undefined) params.set("offset", String(opts.offset));
    return get<SearchResponse>(`${M}/search?${params.toString()}`);
  },
};

// ── System ────────────────────────────────────────────────────

export const system = {
  status: () => get<SystemStatus>("/api/v1/system/status"),
  version: () => get<VersionInfo>("/api/v1/system/version"),
  updateStatus: () => get<UpdateStatus>("/api/v1/system/updates"),
  checkUpdates: () => postJson<UpdateStatus>("/api/v1/system/updates/check"),
  applyUpdate: () => postJson<UpdateStatus>("/api/v1/system/updates/apply"),
  configureUpdates: (config: { mode: "manual" | "automatic"; maintenanceTime: string; timezone: string }) =>
    postJson<UpdateStatus>("/api/v1/system/updates/config", config),
};

// ── Health ────────────────────────────────────────────────────

export const health = {
  check: () => get<HealthResponse>("/health"),
  ready: () => get<string>("/health/ready"),
  live: () => get<string>("/health/live"),
};

// ── EQ ────────────────────────────────────────────────────────

export const eq = {
  get: (zoneId: number) => get<EqConfig>(`${Z}/${zoneId}/eq`),
  set: (zoneId: number, config: EqConfig) => putJson<EqConfig>(`${Z}/${zoneId}/eq`, config),
  setBand: (zoneId: number, idx: number, band: EqBand) => putJson<EqConfig>(`${Z}/${zoneId}/eq/${idx}`, band),
  applyPreset: (zoneId: number, name: string) => postJson<EqConfig>(`${Z}/${zoneId}/eq/preset`, name),
};

export const clientEq = {
  get: (clientId: number) => get<EqConfig>(`${C}/${clientId}/eq`),
  set: (clientId: number, config: EqConfig) => putJson<EqConfig>(`${C}/${clientId}/eq`, config),
  setBand: (clientId: number, idx: number, band: EqBand) => putJson<EqConfig>(`${C}/${clientId}/eq/${idx}`, band),
  applyPreset: (clientId: number, name: string) => postJson<EqConfig>(`${C}/${clientId}/eq/preset`, name),
};

export interface KnxProductInfo {
  app_version: number;
  application_number: string;
  hardware_version: number;
}

const knx = {
  getProgrammingMode: () => get<boolean>("/api/v1/knx/programming-mode"),
  setProgrammingMode: (enabled: boolean) => putJson<boolean>("/api/v1/knx/programming-mode", enabled),
  downloadKnxprod: () => downloadFile("/api/v1/knx/knxprod", "snapdog.knxprod"),
  getProductInfo: () => get<KnxProductInfo>("/api/v1/knx/product-info"),
};

const speakers = {
  list: () => get<string[]>("/api/v1/speakers"),
  getProfile: (name: string) => get<EqConfig>(`/api/v1/speakers/${encodeURIComponent(name)}/profile`),
  apply: (clientId: number, speaker: string | null) => putJson<EqConfig>(`/api/v1/clients/${clientId}/speaker`, { speaker }),
  applyCustom: (clientId: number, custom: EqConfig) => putJson<EqConfig>(`/api/v1/clients/${clientId}/speaker`, { custom }),
  get: (clientId: number) => get<EqConfig>(`/api/v1/clients/${clientId}/speaker`),
};

export const api = { zones, clients, media, system, health, eq, clientEq, knx, speakers };
