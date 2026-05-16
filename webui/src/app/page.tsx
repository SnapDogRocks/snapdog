"use client";

import { useEffect, useCallback, Component, type ReactNode } from "react";
import { useTranslations } from "next-intl";
import { useAppStore, type ZoneState } from "@/stores/useAppStore";
import { useWebSocket } from "@/hooks/useWebSocket";
import { useClientDrop } from "@/hooks/useClientDrop";
import type { WsNotification } from "@/lib/types";
import { ApiKeyPrompt } from "@/components/ApiKeyPrompt";
import { Skeleton } from "@/components/ui/skeleton";
import { ConnectionStatus } from "@/components/ConnectionStatus";
import { LocalePicker } from "@/components/LocalePicker";
import { AboutButton } from "@/components/AboutButton";
import { ProgrammingMode } from "@/components/ProgrammingMode";
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

  const { isConnected: wsConnected, serverGoingAway } = useWebSocket(handleNotification);

  useEffect(() => { setConnected(wsConnected, serverGoingAway); }, [wsConnected, serverGoingAway, setConnected]);
  useEffect(() => { loadAll(); }, [loadAll]);

  const zoneList = Array.from(zoneMap.values());
  const currentZone = zoneMap.get(selectedZone) ?? zoneList[0];

  if (needsAuth) {
    return <ApiKeyPrompt onAuthenticated={() => loadAll()} />;
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
      <ConnectionStatus />
      {/* ── Sidebar / Rail (tablet only) ──────────────────── */}
      <aside className="hidden md:flex xl:hidden flex-col border-r border-border bg-card md:w-56 shrink-0" aria-label={t("zone.navigation")}>
        <div className="px-4 py-4 border-b border-border flex items-center gap-2">
          <img src="/assets/snapdog-icon.svg" alt="" className="size-5 opacity-70" />
          <h1 className="text-base font-semibold">SnapDog</h1>
          <div className="flex items-center gap-0.5 ml-auto">
            <ProgrammingMode /><LocalePicker /><AboutButton />
          </div>
        </div>
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
          <img src="/assets/snapdog-icon.svg" alt="" className="size-5 opacity-70" />
          <h1 className="text-base font-semibold">SnapDog</h1>
          <div className="flex items-center gap-0.5 ml-auto">
            <ProgrammingMode /><LocalePicker /><AboutButton />
          </div>
        </header>

        {/* Wide header (xl+) */}
        <header className="hidden xl:flex items-center gap-2 px-6 py-3 border-b border-border">
          <img src="/assets/snapdog-icon.svg" alt="" className="size-5 opacity-70" />
          <h1 className="text-base font-semibold">SnapDog</h1>
          <div className="flex items-center gap-0.5 ml-auto">
            <ProgrammingMode /><LocalePicker /><AboutButton />
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
