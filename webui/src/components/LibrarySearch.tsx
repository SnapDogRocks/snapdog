"use client";

import { useEffect, useState } from "react";
import { HugeiconsIcon } from "@hugeicons/react";
import { PlayIcon, MusicNote03Icon } from "@hugeicons/core-free-icons";
import { useTranslations } from "next-intl";
import { api, ApiError } from "@/lib/api";
import { logApiError } from "@/lib/log-api-error";
import { formatTime } from "@/lib/format-time";
import type { SearchItem } from "@/lib/types";
import type { ZoneState } from "@/stores/useAppStore";

interface LibrarySearchProps {
  zone: ZoneState;
  query: string;
}

const MIN_QUERY_LENGTH = 2;
const DEBOUNCE_MS = 300;
const RESULTS_PER_KIND = 8;

function Cover({ src }: { src: string | null | undefined }) {
  if (!src) {
    return <HugeiconsIcon icon={MusicNote03Icon} size={24} className="text-primary/30" />;
  }
  return (
    <img
      src={src}
      alt=""
      loading="lazy"
      className="size-full object-cover"
      onError={(e) => {
        if (!e.currentTarget.src.includes("radio-cover.png")) e.currentTarget.src = "/assets/radio-cover.png";
      }}
    />
  );
}

/** Albums and tracks of the media server that match the playlist search text. */
export function LibrarySearch({ zone, query }: LibrarySearchProps) {
  const t = useTranslations("playlist");
  const text = query.trim();
  const active = text.length >= MIN_QUERY_LENGTH;
  const [found, setFound] = useState<{ text: string; items: SearchItem[] } | null>(null);
  const [unavailable, setUnavailable] = useState(false);

  useEffect(() => {
    if (!active) return;
    let stale = false;
    const timer = setTimeout(() => {
      api.media
        .search(text, { kinds: ["album", "track"], limit: RESULTS_PER_KIND })
        .then((res) => {
          if (stale) return;
          setUnavailable(false);
          setFound({ text, items: res.items });
        })
        .catch((e: unknown) => {
          if (stale) return;
          // 503: no media server is configured, so there is nothing to search and nothing to say.
          if (e instanceof ApiError && e.status === 503) setUnavailable(true);
          else logApiError(e);
          setFound(null);
        });
    }, DEBOUNCE_MS);
    return () => {
      stale = true;
      clearTimeout(timer);
    };
  }, [active, text]);

  if (!active || unavailable) return null;
  if (found?.text !== text) {
    return <div className="px-1 text-xs text-muted-foreground animate-pulse">{t("searching")}</div>;
  }

  const albums = found.items.filter((i) => i.kind === "album" && i.album_id);
  const tracks = found.items.filter((i) => i.kind === "track" && i.track_id);
  if (albums.length === 0 && tracks.length === 0) {
    return <div className="px-1 text-xs text-muted-foreground/70">{t("libraryEmpty")}</div>;
  }

  return (
    <div className="space-y-4">
      {albums.length > 0 && (
        <section className="space-y-2">
          <h3 className="px-1 text-sm font-bold tracking-wider text-muted-foreground/70">{t("searchAlbums")}</h3>
          <div className="grid grid-cols-3 @lg:grid-cols-4 @xl:grid-cols-6 gap-3">
            {albums.map((album) => (
              <button
                key={album.album_id}
                title={t("playAlbum")}
                onClick={() => {
                  if (album.album_id) api.zones.playSubsonicAlbum(zone.index, album.album_id).catch(logApiError);
                }}
                className="group space-y-1.5 text-left"
              >
                <div className="w-full aspect-square rounded-xl bg-primary/5 flex items-center justify-center overflow-hidden border border-border/50 group-hover:border-primary/40 transition-all shadow-sm relative">
                  <Cover src={album.cover_art} />
                  <div className="absolute inset-0 bg-black/40 opacity-0 group-hover:opacity-100 transition-opacity flex items-center justify-center">
                    <HugeiconsIcon icon={PlayIcon} size={20} fill="currentColor" className="text-white" />
                  </div>
                </div>
                <div className="px-0.5">
                  <div className="text-sm font-bold truncate group-hover:text-primary transition-colors leading-tight">{album.title}</div>
                  {album.subtitle && <div className="text-xs text-muted-foreground truncate">{album.subtitle}</div>}
                </div>
              </button>
            ))}
          </div>
        </section>
      )}
      {tracks.length > 0 && (
        <section className="space-y-2">
          <h3 className="px-1 text-sm font-bold tracking-wider text-muted-foreground/70">{t("searchTracks")}</h3>
          <div className="space-y-0.5">
            {tracks.map((track) => (
              <button
                key={track.track_id}
                onClick={() => {
                  if (track.track_id) api.zones.playSubsonicTrack(zone.index, track.track_id).catch(logApiError);
                }}
                className="w-full flex items-center gap-3 px-3 py-2 rounded-xl text-left transition-all group hover:bg-muted"
              >
                <div className="size-9 rounded-lg bg-primary/5 flex items-center justify-center shrink-0 overflow-hidden relative border border-border/50">
                  <Cover src={track.cover_art} />
                  <div className="absolute inset-0 bg-black/40 opacity-0 group-hover:opacity-100 flex items-center justify-center transition-opacity">
                    <HugeiconsIcon icon={PlayIcon} size={14} fill="currentColor" className="text-white" />
                  </div>
                </div>
                <div className="min-w-0 flex-1">
                  <div className="text-[13px] font-semibold truncate group-hover:text-primary transition-colors">{track.title}</div>
                  {track.subtitle && <div className="text-[11px] text-muted-foreground truncate">{track.subtitle}</div>}
                </div>
                <span className="text-[10px] tabular-nums text-muted-foreground opacity-60 font-medium">{formatTime(track.duration * 1000)}</span>
              </button>
            ))}
          </div>
        </section>
      )}
    </div>
  );
}
