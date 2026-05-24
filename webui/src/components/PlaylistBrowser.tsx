"use client";

import { useState, useEffect, useMemo } from "react";
import { HugeiconsIcon } from "@hugeicons/react";
import { PlayIcon, MusicNote03Icon, Search01Icon, ArrowLeft01Icon } from "@hugeicons/core-free-icons";
import { useTranslations } from "next-intl";
import { motion, AnimatePresence } from "framer-motion";
import { api } from "@/lib/api";
import { logApiError } from "@/lib/log-api-error";
import { formatTime } from "@/lib/format-time";
import type { PlaylistInfo, TrackInfo } from "@/lib/types";
import type { ZoneState } from "@/stores/useAppStore";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";

interface PlaylistBrowserProps {
  zone: ZoneState;
}

const INITIAL_VISIBLE_COUNT = 6;

export function PlaylistBrowser({ zone }: PlaylistBrowserProps) {
  const t = useTranslations("playlist");
  const [playlists, setPlaylists] = useState<PlaylistInfo[]>([]);
  const [search, setSearch] = useState("");
  const [selectedId, setSelectedId] = useState<number | null>(null);
  const [tracks, setTracks] = useState<TrackInfo[]>([]);
  const [loading, setLoading] = useState(true);
  const [showAll, setShowAll] = useState(false);

  useEffect(() => {
    api.media.playlists()
      .then(setPlaylists)
      .catch(logApiError)
      .finally(() => setLoading(false));
  }, []);

  const filteredPlaylists = useMemo(() => {
    if (!search) return playlists;
    const q = search.toLowerCase();
    return playlists.filter((pl) => pl.name.toLowerCase().includes(q));
  }, [playlists, search]);

  const selectPlaylist = async (id: number) => {
    setSelectedId(id);
    try {
      setTracks(await api.media.tracks(id));
    } catch {
      setTracks([]);
    }
  };

  const playTrack = (playlistId: number, trackIndex: number) => {
    api.zones.playPlaylist(zone.index, playlistId, trackIndex).catch(logApiError);
  };

  const formatDuration = (sec: number) => formatTime(sec * 1000);

  const visiblePlaylists = useMemo(() => {
    if (search || showAll) return filteredPlaylists;
    return filteredPlaylists.slice(0, INITIAL_VISIBLE_COUNT);
  }, [filteredPlaylists, search, showAll]);

  const hasMore = !search && !showAll && filteredPlaylists.length > INITIAL_VISIBLE_COUNT;

  if (loading && playlists.length === 0) {
    return <div className="h-32 flex items-center justify-center text-muted-foreground animate-pulse">{t("loading")}</div>;
  }

  return (
    <div className="w-full space-y-4 pt-2 relative z-10">
      <AnimatePresence mode="wait">
        {selectedId === null ? (
          <motion.div
            key="grid"
            initial={{ opacity: 0, y: 10 }}
            animate={{ opacity: 1, y: 0 }}
            exit={{ opacity: 0, y: -10 }}
            className="space-y-4"
          >
            {/* Header: Title + Search */}
            <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-3 px-1">
              <h3 className="text-sm font-bold tracking-wider text-muted-foreground/70">{t("title")}</h3>
              <div className="relative w-full sm:w-48">
                <HugeiconsIcon icon={Search01Icon} size={14} className="absolute left-2.5 top-1/2 -translate-y-1/2 text-muted-foreground" />
                <Input
                  placeholder={t("search")}
                  value={search}
                  onChange={(e) => setSearch(e.target.value)}
                  className="h-8 pl-8 text-xs bg-muted/30 border-none rounded-lg focus-visible:ring-1 focus-visible:ring-primary/30"
                />
              </div>
            </div>

            {/* Carousel-like Grid */}
            <div className="grid grid-cols-3 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-6 gap-3">
              {visiblePlaylists.map((pl) => (
                <div key={pl.id} className="group relative space-y-1.5">
                  <button
                    onClick={() => selectPlaylist(pl.id)}
                    className="w-full aspect-square rounded-xl bg-primary/5 flex items-center justify-center overflow-hidden border border-border/50 group-hover:border-primary/40 transition-all shadow-sm relative"
                  >
                    {pl.cover_art ? (
                      <img
                        src={pl.cover_art}
                        alt=""
                        loading="lazy"
                        className="size-full object-cover transition-transform duration-500 group-hover:scale-110"
                        onError={(e) => {
                          const img = e.target as HTMLImageElement;
                          if (!img.src.includes('radio-cover.svg')) {
                            img.src = '/assets/radio-cover.svg';
                          }
                        }}
                      />
                    ) : (
                      <HugeiconsIcon icon={MusicNote03Icon} size={28} className="text-primary/30" />
                    )}
                    <div className="absolute inset-0 bg-black/40 opacity-0 group-hover:opacity-100 transition-opacity flex items-center justify-center">
                       <Button
                         size="icon"
                         variant="ghost"
                         className="rounded-full bg-primary text-primary-foreground shadow-lg transform scale-90 group-hover:scale-100 transition-transform hover:bg-primary/90"
                         onClick={(e) => {
                           e.stopPropagation();
                           playTrack(pl.id, 0);
                         }}
                       >
                         <HugeiconsIcon icon={PlayIcon} size={20} fill="currentColor" />
                       </Button>
                    </div>
                  </button>
                  <div className="px-0.5">
                    <div className="text-sm font-bold truncate group-hover:text-primary transition-colors leading-tight">{pl.name}</div>
                    <div className="text-xs text-muted-foreground uppercase tracking-tight">
                      {t("tracks", { count: pl.song_count })}
                    </div>
                  </div>
                </div>
              ))}
            </div>

            {hasMore && (
              <Button
                variant="ghost"
                size="sm"
                onClick={() => setShowAll(true)}
                className="w-full text-[10px] uppercase tracking-widest text-muted-foreground hover:text-primary h-8"
              >
                {t("showMore")} ({playlists.length - INITIAL_VISIBLE_COUNT}+)
              </Button>
            )}
            {showAll && !search && (
              <Button
                variant="ghost"
                size="sm"
                onClick={() => setShowAll(false)}
                className="w-full text-[10px] uppercase tracking-widest text-muted-foreground hover:text-primary h-8"
              >
                {t("showLess")}
              </Button>
            )}

            {filteredPlaylists.length === 0 && (
              <div className="py-12 text-center text-muted-foreground/50 text-sm">
                {search ? t("noResults") : t("empty")}
              </div>
            )}
          </motion.div>
        ) : (
          <motion.div
            key="tracks"
            initial={{ opacity: 0, x: 20 }}
            animate={{ opacity: 1, x: 0 }}
            exit={{ opacity: 0, x: -20 }}
            className="space-y-4"
          >
            {/* Detail Header */}
            <div className="flex items-center gap-3 border-b border-border/50 pb-4 px-1">
              <Button variant="ghost" size="icon" onClick={() => setSelectedId(null)} className="rounded-full size-10 -ml-1 shrink-0">
                <HugeiconsIcon icon={ArrowLeft01Icon} size={20} />
              </Button>
              <div className="size-12 rounded-lg overflow-hidden bg-primary/10 shadow-sm shrink-0">
                 <img
                    src={`/api/v1/media/playlists/${selectedId}/cover`}
                    alt=""
                    className="size-full object-cover"
                    onError={(e) => {
                      const img = e.target as HTMLImageElement;
                      if (!img.src.includes('radio-cover.svg')) {
                        img.src = '/assets/radio-cover.svg';
                      }
                    }}
                  />
              </div>
              <div className="min-w-0 flex-1">
                <h3 className="font-bold truncate text-sm leading-tight">
                  {playlists.find(p => p.id === selectedId)?.name}
                </h3>
                <p className="text-[10px] text-muted-foreground uppercase tracking-wider">
                   {t("tracks", { count: tracks.length })}
                </p>
              </div>
              <Button
                size="sm"
                onClick={() => playTrack(selectedId, 0)}
                className="rounded-full h-8 px-4 gap-1.5"
              >
                <HugeiconsIcon icon={PlayIcon} size={14} fill="currentColor" />
                <span className="text-xs">{t("playAll")}</span>
              </Button>
            </div>

            {/* Track List */}
            <div className="space-y-0.5 max-h-[400px] overflow-y-auto pr-1 scrollbar-thin" ref={(el) => {
              // Auto-scroll to current track
              if (el && zone.track?.playlist_index === selectedId) {
                const idx = zone.track.playlist_track_index;
                if (idx != null) {
                  const child = el.children[idx] as HTMLElement | undefined;
                  child?.scrollIntoView({ block: "nearest", behavior: "smooth" });
                }
              }
            }}>
              {tracks.map((t, i) => {
                const isCurrent = zone.track?.playlist_index === selectedId && zone.track?.playlist_track_index === i;
                return (
                <button
                  key={t.id}
                  onClick={() => playTrack(selectedId, i)}
                  className={`w-full flex items-center gap-3 px-3 py-2 rounded-xl text-left transition-all group ${isCurrent ? "bg-primary/10 ring-1 ring-primary/30" : "hover:bg-muted"}`}
                >
                  <div className="size-9 rounded-lg bg-primary/5 flex items-center justify-center shrink-0 overflow-hidden relative border border-border/50">
                    <img
                      src={t.cover_art || '/assets/radio-cover.svg'}
                      alt=""
                      loading="lazy"
                      className="size-full object-cover"
                      onError={(e) => {
                        const img = e.target as HTMLImageElement;
                        if (!img.src.includes('radio-cover.svg')) {
                          img.src = '/assets/radio-cover.svg';
                        }
                      }}
                    />
                    <div className="absolute inset-0 bg-black/40 opacity-0 group-hover:opacity-100 flex items-center justify-center transition-opacity">
                      <HugeiconsIcon icon={PlayIcon} size={14} fill="currentColor" className="text-white" />
                    </div>
                  </div>
                  <div className="min-w-0 flex-1">
                    <div className="text-[13px] font-semibold truncate group-hover:text-primary transition-colors">{t.title}</div>
                    <div className="text-[11px] text-muted-foreground truncate">{t.artist}</div>
                  </div>
                  <span className="text-[10px] tabular-nums text-muted-foreground opacity-60 font-medium">{formatDuration(t.duration)}</span>
                </button>
              ); })}
            </div>
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}
