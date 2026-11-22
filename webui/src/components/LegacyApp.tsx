"use client";

import { useState, useEffect, useCallback, Component, useRef, type ReactNode } from "react";
import { useTranslations } from "next-intl";
import { useAppStore, type ZoneState } from "@/stores/useAppStore";
import { useAppSession } from "@/hooks/useAppSession";
import { useMediaQuery } from "@/hooks/useMediaQuery";
import { LoadFailure, ApiFeedback } from "@/components/AppFeedback";
import { useClientDrop } from "@/hooks/useClientDrop";
import { ApiKeyPrompt } from "@/components/ApiKeyPrompt";
import { Skeleton } from "@/components/ui/skeleton";
import { ConnectionStatus } from "@/components/ConnectionStatus";
import { LocalePicker } from "@/components/LocalePicker";
import { ThemeToggle } from "@/components/ThemeToggle";
import { AboutButton } from "@/components/AboutButton";
import { ConnectButton } from "@/components/ConnectButton";
import { ZoneRailItem } from "@/components/ZoneRailItem";
import { EmptyState } from "@/components/EmptyState";
import { ZoneDetail } from "@/components/ZoneDetail";

// ── Error Boundary ────────────────────────────────────────────

function ErrorFallback({ error, onRetry }: { error: Error; onRetry: () => void }) {
  const t = useTranslations("zone");
  return (
    <div className="flex flex-1 items-center justify-center p-6 text-center">
      <div className="space-y-2">
        <p className="text-sm font-medium text-destructive">{t("error")}</p>
        <p className="text-xs text-muted-foreground">{error.message}</p>
        <button onClick={onRetry} className="text-xs text-primary-ink hover:underline">{t("retry")}</button>
      </div>
    </div>
  );
}

class ZoneErrorBoundary extends Component<{ children: ReactNode }, { error: Error | null }> {
  override state = { error: null as Error | null };
  static getDerivedStateFromError(error: Error) { return { error }; }
  override render() {
    if (this.state.error) {
      return <ErrorFallback error={this.state.error} onRetry={() => { this.setState({ error: null }); }} />;
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
            ? "text-primary-ink border-b-2 border-primary font-medium"
            : "text-muted-foreground"
      }`}
      aria-current={selected ? "true" : undefined}
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

export function LegacyApp() {
  const t = useTranslations();
  const { zones: zoneMap, selectedZone, selectZone, isLoading, needsAuth, loadAll, loadError } = useAppStore();
  const { serverName, retryIn } = useAppSession();
  const wide = useMediaQuery("(min-width: 1280px)");

  const [activeDot, setActiveDot] = useState(0);
  const carouselRef = useRef<HTMLDivElement>(null);

  const handleScroll = useCallback(() => {
    if (!carouselRef.current) return;
    const container = carouselRef.current;
    const innerWrapper = container.firstElementChild;
    if (!innerWrapper) return;
    const containerCenter = container.getBoundingClientRect().left + container.clientWidth / 2;

    let closestIndex = 0;
    let closestDistance = Infinity;

    let closestChild: HTMLElement | null = null;
    for (const [index, child] of Array.from(innerWrapper.children).entries()) {
      if (!(child instanceof HTMLElement)) continue;
      const childCenter = child.getBoundingClientRect().left + child.clientWidth / 2;
      const distance = Math.abs(containerCenter - childCenter);
      if (distance < closestDistance) {
        closestDistance = distance;
        closestIndex = index;
        closestChild = child;
      }
    }
    setActiveDot(closestIndex);
    const zoneIndex = Number(closestChild?.dataset.zoneIndex);
    const state = useAppStore.getState();
    if (Number.isSafeInteger(zoneIndex) && state.zones.has(zoneIndex) && zoneIndex !== state.selectedZone) {
      selectZone(zoneIndex);
    }
  }, [selectZone]);

  const scrollCarouselTo = (index: number) => {
    carouselRef.current?.firstElementChild?.children.item(index)?.scrollIntoView({
      behavior: "smooth",
      block: "nearest",
      inline: "center",
    });
  };

  const scrollCarousel = (direction: "left" | "right") => {
    const offset = direction === "left" ? -1 : 1;
    scrollCarouselTo(activeDot + offset);
  };

  const zoneList = Array.from(zoneMap.values());
  const currentZone = zoneMap.get(selectedZone) ?? zoneList[0];

  useEffect(() => {
    if (wide && !isLoading) {
      carouselRef.current?.querySelector(`[data-zone-index="${useAppStore.getState().selectedZone}"]`)
        ?.scrollIntoView({ block: "nearest", inline: "center", behavior: "instant" });
    }
  }, [wide, isLoading]);

  if (needsAuth) {
    return <ApiKeyPrompt onAuthenticated={() => { void loadAll(); }} />;
  }

  if (!isLoading && loadError && zoneList.length === 0) return <LoadFailure onRetry={() => { void loadAll(); }} />;

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
    <div className="legacy-app flex flex-1 min-h-0 h-dvh animate-fade-in">
      <a href="#main-content" className="sr-only focus:not-sr-only focus:absolute focus:z-[100] focus:top-2 focus:left-2 focus:px-4 focus:py-2 focus:bg-primary focus:text-primary-foreground focus:rounded-md">
        {t("app.skipToContent")}
      </a>
      <ConnectionStatus retryIn={retryIn} /><ApiFeedback />
      {/* Deprecated since 0.37.0, removed in a later release (docs/upgrading.md). */}
      <p role="note" className="fixed bottom-3 left-1/2 z-50 -translate-x-1/2 rounded-full border border-border bg-card/95 px-4 py-1.5 text-xs shadow-lg backdrop-blur">
        {t("app.legacyDeprecated")}{" "}
        <a href="/" className="font-medium text-primary-ink underline-offset-2 hover:underline">{t("app.legacySwitch")}</a>
      </p>
      {/* ── Sidebar / Rail ──────────────────────────────────
           Visible at md–xl. Hidden at xl+, where the horizontal
           zone carousel below is the primary navigation instead. */}
      <aside className="hidden md:flex flex-col border-r border-border bg-card md:w-56 shrink-0 xl:hidden" aria-label={t("zone.navigation")}>
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
              onSelect={() => { selectZone(z.index); }}
            />
          ))}
        </nav>
      </aside>

      {/* ── Main content ───────────────────────────────────── */}
      <main className="flex flex-1 flex-col min-w-0 min-h-0" id="main-content" tabIndex={-1}>
        <h1 className="sr-only">{serverName}</h1>
        {/* Header (mobile + compact + wide — hidden when sidebar visible at lg–xl) */}
        <header className="flex shrink-0 md:hidden items-center gap-2 px-4 py-3 border-b border-border">
          <div className="flex items-center gap-2">
            <img src="/assets/snapdog-icon.svg" alt="" className="size-8 opacity-70" />

          </div>
          <div className="flex items-center gap-1 ml-auto [&>*]:flex [&>*]:items-center [&>*]:justify-center">
            <AboutButton /><LocalePicker /><ThemeToggle /><ConnectButton />
          </div>
        </header>

        {/* Wide header: shown at xl+, since the sidebar (which otherwise
            carries the logo/controls) is hidden there. */}
        <header className="hidden shrink-0 items-center gap-2 px-6 py-3 border-b border-border xl:flex">
          <div className="flex items-center gap-2">
            <img src="/assets/snapdog-icon.svg" alt="" className="size-8 opacity-70" />

          </div>
          <div className="flex items-center gap-1 ml-auto [&>*]:flex [&>*]:items-center [&>*]:justify-center">
            <AboutButton /><LocalePicker /><ThemeToggle /><ConnectButton />
          </div>
        </header>

        {/* Zone tabs (mobile + compact + normal without sidebar visible) */}
        <div className="flex shrink-0 md:hidden overflow-x-auto border-b border-border px-2 gap-1 scrollbar-none" aria-label={t("zone.zones")}>
          {zoneList.map((z) => (
            <MobileZoneTab key={z.index} zone={z} selected={z.index === selectedZone} onSelect={() => { selectZone(z.index); }} />
          ))}
        </div>

        {/* Desktop: all zones in elegant horizontal snap-scroll carousel (xl+) */}
        {wide && <div className="flex flex-col flex-1 min-h-0 relative bg-gradient-to-b from-background/10 via-background/5 to-background/2">
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
                    <div data-zone-index={z.index} onFocusCapture={() => { selectZone(z.index); }} onPointerDownCapture={() => { selectZone(z.index); }} className="relative w-[480px] xl:w-[520px] h-[calc(100vh-160px)] max-h-[780px] shrink-0 snap-center snap-always transition-all duration-300 group">
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
              onClick={() => { scrollCarousel("left"); }}
              className="absolute left-6 top-1/2 -translate-y-1/2 size-12 rounded-full bg-black/60 hover:bg-black/80 border border-white/10 flex items-center justify-center text-white transition-all shadow-xl z-20 group hover:scale-105"
              aria-label={t("current.previousZone")}
            >
              <svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round" className="group-hover:-translate-x-0.5 transition-transform"><polyline points="15 18 9 12 15 6"/></svg>
            </button>
          )}

          {/* Right Navigation Arrow */}
          {activeDot < zoneList.length - 1 && (
            <button
              onClick={() => { scrollCarousel("right"); }}
              className="absolute right-6 top-1/2 -translate-y-1/2 size-12 rounded-full bg-black/60 hover:bg-black/80 border border-white/10 flex items-center justify-center text-white transition-all shadow-xl z-20 group hover:scale-105"
              aria-label={t("current.nextZone")}
            >
              <svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round" className="group-hover:translate-x-0.5 transition-transform"><polyline points="9 18 15 12 9 6"/></svg>
            </button>
          )}

          {/* Bottom Indicators (Dots) */}
          <div className="flex justify-center gap-2 pb-6">
            {zoneList.map((z, idx) => (
              <button
                key={z.index}
                onClick={() => { scrollCarouselTo(idx); }}
                className={`h-6 min-w-6 rounded-full transition-all duration-300 ${idx === activeDot ? "w-8 bg-primary" : "w-6 bg-muted-foreground/30 hover:bg-muted-foreground/60"}`}
                aria-label={z.name}
              />
            ))}
          </div>
        </div>}

        {/* Only mount the visible layout, so hidden players cannot fetch or mutate. */}
        {!wide && <div className="flex flex-col flex-1 min-h-0 overflow-y-auto">
          {currentZone && (
            <ZoneErrorBoundary>
              <ZoneDetail zone={currentZone} />
            </ZoneErrorBoundary>
          )}
        </div>}
      </main>
    </div>
  );
}
