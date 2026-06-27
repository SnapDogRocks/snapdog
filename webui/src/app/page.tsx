"use client";

import { useState, useEffect, useCallback, Component, useRef, type ReactNode } from "react";
import { useTranslations } from "next-intl";
import { useAppStore, type ZoneState } from "@/stores/useAppStore";
import { useWebSocket } from "@/hooks/useWebSocket";
import { useKeyboardShortcuts } from "@/hooks/useKeyboardShortcuts";
import { useClientDrop } from "@/hooks/useClientDrop";
import type { WsNotification } from "@/lib/types";
import { ApiKeyPrompt } from "@/components/ApiKeyPrompt";
import { api } from "@/lib/api";
import { logApiError } from "@/lib/log-api-error";
import { Skeleton } from "@/components/ui/skeleton";
import { ConnectionStatus } from "@/components/ConnectionStatus";
import { LocalePicker } from "@/components/LocalePicker";
import { ThemeToggle } from "@/components/ThemeToggle";
import { AboutButton } from "@/components/AboutButton";
import { ConnectButton } from "@/components/ConnectButton";
import { ZoneRailItem } from "@/components/ZoneRailItem";
import { ZoneDetail } from "@/components/ZoneDetail";

// ── Error Boundary ────────────────────────────────────────────

function ErrorFallback({ error, onRetry }: { error: Error; onRetry: () => void }) {
  const t = useTranslations("zone");
  return (
    <div className="flex flex-1 items-center justify-center p-6 text-center">
      <div className="space-y-2">
        <p className="text-sm font-medium text-destructive">{t("error")}</p>
        <p className="text-xs text-muted-foreground">{error.message}</p>
        <button onClick={onRetry} className="text-xs text-primary hover:underline">{t("retry")}</button>
      </div>
    </div>
  );
}

class ZoneErrorBoundary extends Component<{ children: ReactNode }, { error: Error | null }> {
  state = { error: null as Error | null };
  static getDerivedStateFromError(error: Error) { return { error }; }
  render() {
    if (this.state.error) {
      return <ErrorFallback error={this.state.error} onRetry={() => this.setState({ error: null })} />;
    }
    return this.props.children;
  }
}

// ── Mobile Zone Tab ───────────────────────────────────────────

function EmptyState() {
  const t = useTranslations("empty");
  const [progMode, setProgMode] = useState(false);
  const [knxAvailable, setKnxAvailable] = useState(true);

  useEffect(() => {
    api.knx.getProgrammingMode()
      .then(setProgMode)
      .catch(() => setKnxAvailable(false));
  }, []);

  const toggleProg = () => {
    const next = !progMode;
    api.knx.setProgrammingMode(next)
      .then(() => setProgMode(next))
      .catch(logApiError);
  };

  return (
    <div className="fixed inset-0 flex min-w-0 items-center justify-center overflow-hidden px-6 py-10">
      <div className="mx-auto flex w-full max-w-xs min-w-0 flex-col items-center space-y-6 text-center">
        <div className="animate-pulse-slow">
          <img src="/assets/snapdog-icon.svg" alt="SnapDog" className="size-24 mx-auto opacity-40" />
        </div>
        <div className="space-y-2">
          <h2 className="text-lg font-semibold">{t("title")}</h2>
          <p className="mx-auto max-w-[18rem] text-sm leading-relaxed text-muted-foreground">{t("description")}</p>
        </div>
        {knxAvailable && (
          <div className="flex items-center justify-center gap-3">
            <span className="text-sm text-muted-foreground">{t("programmingMode")}</span>
            <button
              onClick={toggleProg}
              className={`relative w-11 h-6 rounded-full transition-colors ${progMode ? "bg-red-500" : "bg-muted"}`}
              role="switch"
              aria-checked={progMode}
            >
              <span className={`absolute top-0.5 left-0.5 size-5 rounded-full bg-white shadow transition-transform ${progMode ? "translate-x-5" : ""}`} />
            </button>
          </div>
        )}
      </div>
    </div>
  );
}

function MobileZoneTab({ zone, selected, onSelect }: { zone: ZoneState; selected: boolean; onSelect: () => void }) {
  const { dragOver, dragHandlers } = useClientDrop(zone.index);
  return (
    <button
      onClick={onSelect}
      {...dragHandlers}
      className={`shrink-0 px-3 py-2 text-sm rounded-t-md transition-colors ${
        dragOver
          ? "bg-primary/20 ring-2 ring-primary"
          : selected
            ? "text-primary border-b-2 border-primary font-medium"
            : "text-muted-foreground"
      }`}
      role="tab"
      aria-selected={selected}
    >
      {zone.name}
    </button>
  );
}

// ── Zone Drop Target ──────────────────────────────────────────

function ZoneDropTarget({ zoneIndex, children }: { zoneIndex: number; children: ReactNode }) {
  const { dragOver, dragHandlers } = useClientDrop(zoneIndex);

  return (
    <div
      className={`flex flex-col h-full border rounded-[2rem] bg-card/45 border-border/40 backdrop-blur-xl shadow-2xl transition-all duration-300 overflow-hidden ${
        dragOver ? "border-primary ring-4 ring-primary/15" : "hover:border-border/60 hover:shadow-[0_25px_60px_rgba(0,0,0,0.35)]"
      }`}
      {...dragHandlers}
    >
      {children}
    </div>
  );
}

// ── App Shell ─────────────────────────────────────────────────

export default function Home() {
  const t = useTranslations();
  const {
    zones: zoneMap,
    selectedZone,
    selectZone,
    isLoading,
    needsAuth,
    setConnected,
    loadAll,
    updateZone,
    updateZoneVolume,
    updateZoneProgress,
    updateZonePresence,
    updateClient,
    setZoneError,
  } = useAppStore();

  const [activeDot, setActiveDot] = useState(0);
  const carouselRef = useRef<HTMLDivElement>(null);

  const handleScroll = useCallback(() => {
    if (!carouselRef.current) return;
    const container = carouselRef.current;
    const innerWrapper = container.firstElementChild;
    if (!innerWrapper) return;
    const containerCenter = container.scrollLeft + container.clientWidth / 2;
    
    let closestIndex = 0;
    let closestDistance = Infinity;
    
    const children = innerWrapper.children;
    for (let i = 0; i < children.length; i++) {
      const child = children[i] as HTMLElement;
      const childCenter = child.offsetLeft + child.clientWidth / 2;
      const distance = Math.abs(containerCenter - childCenter);
      if (distance < closestDistance) {
        closestDistance = distance;
        closestIndex = i;
      }
    }
    setActiveDot(closestIndex);
  }, []);

  const scrollCarousel = (direction: "left" | "right") => {
    if (carouselRef.current) {
      const el = carouselRef.current;
      const innerWrapper = el.firstElementChild;
      if (!innerWrapper) return;
      const targetIndex = direction === "left" ? activeDot - 1 : activeDot + 1;
      const children = innerWrapper.children;
      if (children[targetIndex]) {
        (children[targetIndex] as HTMLElement).scrollIntoView({ behavior: "smooth", block: "nearest", inline: "center" });
      }
    }
  };

  const handleNotification = useCallback(
    (n: WsNotification) => {
      switch (n.type) {
        case "zone_changed":
          updateZone(n.zone, {
            playback: n.playback,
            volume: n.volume,
            muted: n.muted,
            source: n.source,
            shuffle: n.shuffle,
            repeat: n.repeat,
            track: {
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
              playlist_index: n.playlist,
              playlist_name: n.playlist_name,
              playlist_total: n.playlist_total,
              playlist_track_index: n.track_index,
              playlist_track_count: n.track_count,
              can_playlist_next: n.can_playlist_next,
              can_playlist_prev: n.can_playlist_prev,
              can_next: n.can_next,
              can_prev: n.can_prev,
            },
          });
          break;
        case "zone_volume_changed":
          updateZoneVolume(n.zone, n.volume, n.muted);
          break;
        case "zone_progress":
          updateZoneProgress(n.zone, n.position_ms, n.duration_ms, n.buffered_ms ?? null);
          break;
        case "client_state_changed":
          updateClient(n.client, {
            volume: n.volume,
            muted: n.muted,
            connected: n.connected,
            zone_index: n.zone,
            is_snapdog: n.is_snapdog,
          });
          break;
        case "zone_presence_changed":
          updateZonePresence(n.zone, n.presence, n.enabled, n.timer_active);
          break;
        case "zone_eq_changed":
          useAppStore.getState().updateZoneEq(n.zone, n.enabled, n.bands, n.preset);
          break;
        case "playback_error":
          setZoneError(n.zone, {
            message: n.message,
            details: n.details,
            recoverable: n.recoverable,
          });
          break;
      }
    },
    [updateZone, updateZoneVolume, updateZoneProgress, updateZonePresence, updateClient, setZoneError],
  );

  const { isConnected: wsConnected, serverGoingAway, retryIn } = useWebSocket(handleNotification);
  useKeyboardShortcuts();

  useEffect(() => { setConnected(wsConnected, serverGoingAway); }, [wsConnected, serverGoingAway, setConnected]);
  useEffect(() => { loadAll(); }, [loadAll]);

  // Set document title from server name
  const [serverName, setServerName] = useState("SnapDog");
  useEffect(() => {
    api.system.version().then((v) => {
      setServerName(v.name);
      document.title = v.name === "SnapDog" ? "SnapDog" : `SnapDog — ${v.name}`;
    }).catch(() => {});
  }, []);

  // Handle ?auth= URL parameter (from shared links)
  useEffect(() => {
    if (typeof window === "undefined") return;
    const params = new URLSearchParams(window.location.search);
    const authParam = params.get("auth");
    if (authParam) {
      sessionStorage.setItem("snapdog_api_key", authParam);
      params.delete("auth");
      const clean = params.toString();
      const url = clean ? `${window.location.pathname}?${clean}` : window.location.pathname;
      window.history.replaceState({}, "", url);
      loadAll();
    }
  }, [loadAll]);

  const zoneList = Array.from(zoneMap.values());
  const currentZone = zoneMap.get(selectedZone) ?? zoneList[0];

  // The sidebar is hidden at xl to show the elegant horizontal zone carousel
  const allZonesFitInGrid = true;

  if (needsAuth) {
    return <ApiKeyPrompt onAuthenticated={() => loadAll()} />;
  }

  if (!isLoading && zoneList.length === 0) {
    return <EmptyState />;
  }

  if (isLoading) {
    return (
      <div className="flex flex-1 h-full">
        {/* Skeleton sidebar */}
        <aside className="hidden md:flex xl:hidden flex-col border-r border-border bg-card md:w-56 shrink-0">
          <div className="px-4 py-4 border-b border-border">
            <Skeleton className="h-5 w-24" />
          </div>
          <div className="p-2 space-y-2">
            {[1, 2, 3].map((i) => (
              <div key={i} className="flex items-center gap-3 px-3 py-3">
                <Skeleton className="size-10 rounded-md" />
                <div className="flex-1 space-y-1.5">
                  <Skeleton className="h-3.5 w-24" />
                  <Skeleton className="h-3 w-32" />
                </div>
              </div>
            ))}
          </div>
        </aside>
        {/* Skeleton main */}
        <main className="flex flex-1 flex-col items-center justify-center gap-5 p-6">
          <Skeleton className="w-full max-w-xs aspect-square rounded-2xl" />
          <Skeleton className="h-5 w-40" />
          <Skeleton className="h-4 w-28" />
          <Skeleton className="h-10 w-48 rounded-full" />
        </main>
      </div>
    );
  }

  return (
    <div className="flex flex-1 h-full animate-fade-in">
      <a href="#main-content" className="sr-only focus:not-sr-only focus:absolute focus:z-[100] focus:top-2 focus:left-2 focus:px-4 focus:py-2 focus:bg-primary focus:text-primary-foreground focus:rounded-md">
        {t("app.skipToContent")}
      </a>
      <ConnectionStatus retryIn={retryIn} />
      {/* ── Sidebar / Rail ──────────────────────────────────
           Visible at md+. Hides at xl only when all zones fit
           simultaneously in the wide grid (≤ 2 zones). When there
           are 3+ zones, the grid overflows so we keep the sidebar
           as the primary navigation at all viewport sizes. */}
      <aside className={`hidden md:flex flex-col border-r border-border bg-card md:w-56 shrink-0${allZonesFitInGrid ? ' xl:hidden' : ''}`} aria-label={t("zone.navigation")}>
        <div className="px-4 py-4 border-b border-border flex items-center gap-2">
          <div className="flex items-center gap-2">
            <img src="/assets/snapdog-icon.svg" alt="" className="size-8 opacity-70" />
            
          </div>
          <div className="flex items-center gap-1 ml-auto [&>*]:flex [&>*]:items-center [&>*]:justify-center">
            <AboutButton /><LocalePicker /><ThemeToggle /><ConnectButton />
          </div>
        </div>
        {serverName !== "SnapDog" && (
          <div className="px-4 pt-3 pb-1 text-[11px] font-semibold uppercase tracking-wider text-muted-foreground/50">
            {serverName}
          </div>
        )}
        <nav className="flex-1 overflow-y-auto p-2 space-y-0.5" aria-label={t("zone.zones")}>
          {zoneList.map((z) => (
            <ZoneRailItem
              key={z.index}
              zone={z}
              selected={z.index === selectedZone}
              onSelect={() => selectZone(z.index)}
            />
          ))}
        </nav>
      </aside>

      {/* ── Main content ───────────────────────────────────── */}
      <main className="flex flex-1 flex-col min-w-0" id="main-content">
        {/* Header (mobile + compact + wide — hidden when sidebar visible at lg–xl) */}
        <header className="flex md:hidden items-center gap-2 px-4 py-3 border-b border-border">
          <div className="flex items-center gap-2">
            <img src="/assets/snapdog-icon.svg" alt="" className="size-8 opacity-70" />
            
          </div>
          <div className="flex items-center gap-1 ml-auto [&>*]:flex [&>*]:items-center [&>*]:justify-center">
            <AboutButton /><LocalePicker /><ThemeToggle /><ConnectButton />
          </div>
        </header>

        {/* Wide header (xl+) */}
        {/* Wide header: only shown at xl when the sidebar is hidden (≤ 2 zones).
            When sidebar is visible (3+ zones) it already contains the logo/controls. */}
        <header className={`hidden items-center gap-2 px-6 py-3 border-b border-border${allZonesFitInGrid ? ' xl:flex' : ''}`}>
          <div className="flex items-center gap-2">
            <img src="/assets/snapdog-icon.svg" alt="" className="size-8 opacity-70" />
            
          </div>
          <div className="flex items-center gap-1 ml-auto [&>*]:flex [&>*]:items-center [&>*]:justify-center">
            <AboutButton /><LocalePicker /><ThemeToggle /><ConnectButton />
          </div>
        </header>

        {/* Zone tabs (mobile + compact + normal without sidebar visible) */}
        <div className="flex lg:hidden overflow-x-auto border-b border-border px-2 gap-1 scrollbar-none" role="tablist" aria-label={t("zone.zones")}>
          {zoneList.map((z) => (
            <MobileZoneTab key={z.index} zone={z} selected={z.index === selectedZone} onSelect={() => selectZone(z.index)} />
          ))}
        </div>

        {/* Desktop: all zones in elegant horizontal snap-scroll carousel (xl+) */}
        <div className="hidden xl:flex flex-col flex-1 min-h-0 relative bg-gradient-to-b from-background/10 via-background/5 to-background/2">
          {/* Scrollable Container */}
          <div
            ref={carouselRef}
            onScroll={handleScroll}
            className="flex flex-1 overflow-x-auto scrollbar-none scroll-smooth snap-x snap-mandatory py-10 px-6 min-w-0"
          >
            <div className="flex items-center gap-10 mx-auto min-w-max px-16 h-full">
              {zoneList.map((z) => {
                const glowUrl = z.source !== "idle" ? (z.track?.cover_url ?? null) : null;
                return (
                  <ZoneErrorBoundary key={z.index}>
                    <div className="relative w-[480px] xl:w-[520px] h-[calc(100vh-160px)] max-h-[780px] shrink-0 snap-center snap-always transition-all duration-300 group">
                      {/* Ambient outer glow */}
                      {glowUrl && (
                        <div className="absolute -inset-6 pointer-events-none select-none z-0 opacity-20 blur-3xl transition-opacity duration-1000 group-hover:opacity-35" aria-hidden="true">
                          <img
                            src={glowUrl}
                            alt=""
                            className="size-full object-cover rounded-full scale-[0.85]"
                          />
                        </div>
                      )}
                      <div className="relative z-10 h-full">
                        <ZoneDropTarget zoneIndex={z.index}>
                          <ZoneDetail zone={z} />
                        </ZoneDropTarget>
                      </div>
                    </div>
                  </ZoneErrorBoundary>
                );
              })}
            </div>
          </div>

          {/* Left Navigation Arrow */}
          {activeDot > 0 && (
            <button
              onClick={() => scrollCarousel("left")}
              className="absolute left-6 top-1/2 -translate-y-1/2 size-12 rounded-full bg-black/60 hover:bg-black/80 border border-white/10 flex items-center justify-center text-white transition-all shadow-xl z-20 group hover:scale-105"
              aria-label="Previous Zone"
            >
              <svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round" className="group-hover:-translate-x-0.5 transition-transform"><polyline points="15 18 9 12 15 6"/></svg>
            </button>
          )}

          {/* Right Navigation Arrow */}
          {activeDot < zoneList.length - 1 && (
            <button
              onClick={() => scrollCarousel("right")}
              className="absolute right-6 top-1/2 -translate-y-1/2 size-12 rounded-full bg-black/60 hover:bg-black/80 border border-white/10 flex items-center justify-center text-white transition-all shadow-xl z-20 group hover:scale-105"
              aria-label="Next Zone"
            >
              <svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round" className="group-hover:translate-x-0.5 transition-transform"><polyline points="9 18 15 12 9 6"/></svg>
            </button>
          )}

          {/* Bottom Indicators (Dots) */}
          <div className="flex justify-center gap-2 pb-6">
            {zoneList.map((z, idx) => (
              <button
                key={z.index}
                onClick={() => {
                  if (carouselRef.current) {
                    const el = carouselRef.current;
                    const innerWrapper = el.firstElementChild;
                    if (innerWrapper) {
                      const children = innerWrapper.children;
                      if (children[idx]) {
                        (children[idx] as HTMLElement).scrollIntoView({ behavior: "smooth", block: "nearest", inline: "center" });
                      }
                    }
                  }
                }}
                className={`h-2 rounded-full transition-all duration-300 ${idx === activeDot ? "w-6 bg-primary" : "w-2 bg-muted-foreground/30 hover:bg-muted-foreground/60"}`}
                aria-label={`Go to zone ${z.name}`}
              />
            ))}
          </div>
        </div>

        {/* Mobile/Tablet: single selected zone */}
        <div className="xl:hidden flex-1">
          {currentZone && (
            <ZoneErrorBoundary>
              <ZoneDetail zone={currentZone} />
            </ZoneErrorBoundary>
          )}
        </div>
      </main>
    </div>
  );
}
