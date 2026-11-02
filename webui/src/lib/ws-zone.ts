import type { TrackMetadata, WsZoneChanged } from "./types";

/**
 * Track part of a `zone_changed` notification. The deprecated `playlist`
 * field is never read, `playlist_index` and `playlist_id` carry the identity.
 */
export function trackFromZoneChanged(
  n: WsZoneChanged,
): Omit<TrackMetadata, "sample_rate" | "source"> {
  return {
    title: n.title,
    artist: n.artist,
    album: n.album,
    album_artist: n.album_artist,
    genre: n.genre,
    year: n.year,
    track_number: n.track_number,
    disc_number: n.disc_number,
    duration_ms: n.duration_ms,
    position_ms: n.position_ms,
    seekable: n.seekable,
    cover_url: n.cover_url,
    bitrate_kbps: n.bitrate_kbps,
    content_type: n.content_type,
    playlist_index: n.playlist_index,
    playlist_id: n.playlist_id,
    playlist_name: n.playlist_name,
    playlist_total: n.playlist_total,
    playlist_track_index: n.track_index,
    playlist_track_count: n.track_count,
    can_playlist_next: n.can_playlist_next,
    can_playlist_prev: n.can_playlist_prev,
    can_next: n.can_next,
    can_prev: n.can_prev,
  };
}
