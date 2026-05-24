// ── Enums ─────────────────────────────────────────────────────

export type PlaybackState = "playing" | "paused" | "stopped";

export type SourceType =
  | "idle"
  | "radio"
  | "subsonic_playlist"
  | "subsonic_track"
  | "url"
  | "airplay"
  | "spotify";

// ── Zone ──────────────────────────────────────────────────────

export interface ZoneInfo {
  index: number;
  name: string;
  icon: string;
  volume: number;
  muted: boolean;
  playback: PlaybackState;
  source: SourceType;
  shuffle: boolean;
  repeat: boolean;
  track_repeat: boolean;
  presence: boolean;
  presence_enabled: boolean;
  presence_timer_active: boolean;
}

export interface TrackMetadata {
  title: string;
  artist: string;
  album: string;
  album_artist: string | null;
  genre: string | null;
  year: number | null;
  track_number: number | null;
  disc_number: number | null;
  duration_ms: number;
  position_ms: number;
  seekable: boolean;
  bitrate_kbps: number | null;
  content_type: string | null;
  sample_rate: number | null;
  source: string;
  /** Injected from zone state by the API, not from the track source. */
  cover_url: string | null;
  playlist_index: number | null;
  playlist_track_index: number | null;
  playlist_track_count: number | null;
}

export interface PlaylistState {
  index: number | null;
  name: string | null;
  track_index: number | null;
  track_count: number | null;
}

// ── Client ────────────────────────────────────────────────────

export interface ClientInfo {
  index: number;
  name: string;
  mac: string;
  zone_index: number;
  icon: string;
  volume: number;
  max_volume: number;
  muted: boolean;
  connected: boolean;
  is_snapdog: boolean;
}

// ── Media (Subsonic) ──────────────────────────────────────────

export interface PlaylistInfo {
  id: number;
  name: string;
  song_count: number;
  duration: number;
  cover_art: string | null;
}

export interface TrackInfo {
  id: string;
  title: string;
  artist: string;
  album: string;
  duration: number;
  track?: number;
  cover_art?: string | null;
}


// ── System ────────────────────────────────────────────────────

export interface SystemStatus {
  version: string;
  zones: number;
  clients: number;
  radios: number;
}

export interface VersionInfo {
  version: string;
  rust_version: string;
  name: string;
}

export interface HealthResponse {
  status: string;
  zones: number;
  clients: number;
}

// ── WebSocket ─────────────────────────────────────────────────

export interface WsZoneStateChanged {
  type: "zone_state_changed";
  zone: number;
  playback: PlaybackState;
  volume: number;
  muted: boolean;
  source: SourceType;
  shuffle: boolean;
  repeat: boolean;
  track_repeat: boolean;
}

export interface WsZoneTrackChanged {
  type: "zone_track_changed";
  zone: number;
  title: string;
  artist: string;
  album: string;
  duration_ms: number;
  position_ms: number;
  seekable: boolean;
  cover_url: string | null;
}

export interface WsZoneProgress {
  type: "zone_progress";
  zone: number;
  position_ms: number;
  duration_ms: number;
  buffered_ms?: number;
}

export interface WsClientStateChanged {
  type: "client_state_changed";
  client: number;
  volume: number;
  muted: boolean;
  connected: boolean;
  zone: number;
  is_snapdog: boolean;
}

export interface WsZonePresenceChanged {
  type: "zone_presence_changed";
  zone: number;
  presence: boolean;
  enabled: boolean;
  timer_active: boolean;
}

export interface WsZoneEqChanged {
  type: "zone_eq_changed";
  zone: number;
  enabled: boolean;
  bands: Array<{ filter_type: string; frequency: number; gain: number; q: number }>;
  preset?: string;
}

export type WsNotification =
  | WsZoneStateChanged
  | WsZoneTrackChanged
  | WsZoneProgress
  | WsClientStateChanged
  | WsZonePresenceChanged
  | WsZoneEqChanged;

export interface WsCommand {
  zone: number;
  action: string;
  value?: string | number | boolean;
}

// ── Volume ────────────────────────────────────────────────────

export type VolumeValue = number | string; // absolute (75) or relative ("+5")

// ── EQ ────────────────────────────────────────────────────────

export interface EqBand {
  freq: number;
  gain: number;
  q: number;
  type: "low_shelf" | "high_shelf" | "peaking" | "low_pass" | "high_pass";
}

export interface EqConfig {
  enabled: boolean;
  bands: EqBand[];
  preset?: string | null;
}
