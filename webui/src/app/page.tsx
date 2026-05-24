"use client";

import { useState, useEffect, useCallback, Component, type ReactNode } from "react";
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
    <div className="flex flex-1 items-center justify-center p-8">
      <div className="max-w-sm text-center space-y-6">
        <div className="animate-pulse-slow">
          <img src="/assets/snapdog-icon.svg" alt="SnapDog" className="size-24 mx-auto opacity-40" />
        </div>
        <div className="space-y-2">
          <h2 className="text-lg font-semibold">{t("title")}</h2>
          <p className="text-sm text-muted-foreground">{t("description")}</p>
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
      className={`border rounded-xl bg-card overflow-hidden transition-colors ${dragOver ? "border-primary border-2" : "border-border"}`}
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
    updateZoneTrack,
    updateZoneProgress,
    updateZonePresence,
    updateClient,
  } = useAppStore();

  const handleNotification = useCallback(
    (n: WsNotification) => {
      switch (n.type) {
        case "zone_state_changed":
          updateZone(n.zone, {
            playback: n.playback,
            volume: n.volume,
            muted: n.muted,
            source: n.source,
            shuffle: n.shuffle,
            repeat: n.repeat,
            track_repeat: n.track_repeat,
          });
          break;
        case "zone_track_changed":
          updateZoneTrack(n.zone, { ...n, cover_url: n.cover_url });
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
      }
    },
    [updateZone, updateZoneTrack, updateZoneProgress, updateZonePresence, updateClient],
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

  // At the xl breakpoint (1280px), the wide grid uses minWidth=480px cards with gap=16px.
  // Available width ≈ 1280 - 32px padding = 1248px.
  // n cards fit without scroll when: n*480 + (n-1)*16 ≤ 1248  →  n ≤ 2
  // When all zones fit, the sidebar is redundant and should hide at xl.
  // When there are 3+ zones, the grid overflows horizontally and the sidebar
  // must remain visible so the user can navigate to off-screen zones.
  const allZonesFitInGrid = zoneList.length <= 2;

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

        {/* Desktop: all zones in responsive grid (xl+) */}
        <div className="hidden xl:flex xl:flex-wrap xl:gap-4 xl:p-4 xl:justify-center flex-1 overflow-y-auto">
          {zoneList.map((z) => (
            <ZoneErrorBoundary key={z.index}>
              <div className="w-full" style={{ minWidth: '480px', maxWidth: '600px', flex: '1 1 480px' }}>
                <ZoneDropTarget zoneIndex={z.index}>
                  <ZoneDetail zone={z} />
                </ZoneDropTarget>
              </div>
            </ZoneErrorBoundary>
          ))}
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
