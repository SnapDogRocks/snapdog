"use client";

import { useState } from "react";
import { useTranslations } from "next-intl";
import { HugeiconsIcon } from "@hugeicons/react";
import { AlertCircleIcon, Cancel01Icon } from "@hugeicons/core-free-icons";
import { api } from "@/lib/api";
import { logApiError } from "@/lib/log-api-error";
import { useAppStore, type ZoneState } from "@/stores/useAppStore";
import type { SourceType } from "@/lib/types";
import { NowPlaying } from "@/components/NowPlaying";
import { TransportControls } from "@/components/TransportControls";
import { EqOverlay } from "@/components/EqOverlay";
import { Button } from "@/components/ui/button";
import { VolumeSlider } from "@/components/VolumeSlider";
import { SeekBar } from "@/components/SeekBar";
import { ShuffleRepeat } from "@/components/ShuffleRepeat";
import { PlaylistBrowser } from "@/components/PlaylistBrowser";
import { ClientList } from "@/components/ClientList";
import { Marquee } from "@/components/Marquee";

const SOURCE_KEYS: Partial<Record<SourceType, string>> = {
  radio: "radio",
  subsonic_playlist: "subsonic_playlist",
  subsonic_track: "subsonic_track",
  airplay: "airplay",
  spotify: "spotify",
  url: "url",
};

function ZoneHeader({ zone }: { zone: ZoneState }) {
  const t = useTranslations();
  const sourceKey = SOURCE_KEYS[zone.source];
  return (
    <div className="flex items-center justify-between gap-2">
      <div className="flex items-center gap-2 truncate">
        <h2 className="text-sm font-semibold truncate">{zone.name}</h2>
        {zone.eqEnabled && (
          <span className="text-[8px] font-bold text-primary bg-primary/10 px-1 py-0.5 rounded uppercase tracking-wider shrink-0" aria-label="EQ Active">
            EQ
          </span>
        )}
      </div>
      <div className="flex items-center gap-1.5 shrink-0">
        {zone.presence && (
          <span
            className="text-[10px] px-1 py-0.5 rounded-full bg-green-500/15 text-green-600"
            role="status"
            aria-label={zone.presenceTimerActive ? t("zone.presenceTimerActive") : t("zone.presenceDetected")}
          >
            {zone.presenceTimerActive ? "⏱️" : "🟢"}
          </span>
        )}
        {sourceKey ? (
        <span className="text-[10px] font-medium uppercase tracking-wider px-1.5 py-0.5 rounded-full bg-primary/10 text-primary">
          {t(`source.${sourceKey}`)}
        </span>
      ) : (
        <span className="text-[10px] text-muted-foreground">{t("zone.idle")}</span>
      )}
      </div>
    </div>
  );
}

function TrackInfo({ zone, forceCenter = false }: { zone: ZoneState; forceCenter?: boolean }) {
  const t = useTranslations("zone");
  const track = zone.track;
  const isIdle = zone.source === "idle" || !track;

  const alignmentClass = forceCenter ? "text-center" : "text-center sm:text-left";

  if (isIdle) {
    return (
      <div className={`${alignmentClass} w-full flex flex-col justify-start`}>
        <div className="text-base font-bold leading-snug">{t("noAudio")}</div>
        <div className="text-sm text-muted-foreground mt-0.5">{"\u00A0"}</div>
        <div className="text-xs text-muted-foreground/70 mt-0.5">{"\u00A0"}</div>
      </div>
    );
  }

  return (
    <div className={`${alignmentClass} w-full flex flex-col justify-start`}>
      <Marquee className="text-base font-bold leading-snug">{track.title || t("unknownTitle")}</Marquee>
      <Marquee className="text-sm text-muted-foreground mt-0.5">{track.artist || t("unknownArtist")}</Marquee>
      <Marquee className="text-xs text-muted-foreground/70 mt-0.5">{track.album || "\u00A0"}</Marquee>
    </div>
  );
}

function PlaybackErrorBanner({ zone }: { zone: ZoneState }) {
  if (!zone.error) return null;

  return (
    <div
      className="flex items-start gap-3 rounded-lg border border-destructive/25 bg-destructive/10 px-3.5 py-3 text-sm text-destructive shadow-sm"
      role="alert"
      aria-live="polite"
    >
      <HugeiconsIcon icon={AlertCircleIcon} size={18} className="mt-0.5 shrink-0" />
      <div className="min-w-0 flex-1 space-y-1">
        <p className="break-words font-medium leading-snug">{zone.error.message}</p>
        {zone.error.details && (
          <details>
            <summary className="cursor-pointer select-none text-[11px] font-medium text-destructive/80 hover:text-destructive">
              Technical details
            </summary>
            <pre className="mt-1.5 max-h-28 overflow-auto whitespace-pre-wrap break-all rounded-md border border-destructive/15 bg-background/70 p-2 font-mono text-[11px] leading-relaxed text-destructive/85">
              {zone.error.details}
            </pre>
          </details>
        )}
      </div>
      <Button
        variant="ghost"
        size="icon"
        onClick={() => useAppStore.getState().setZoneError(zone.index, null)}
        className="-mr-1 -mt-1 size-7 shrink-0 rounded-full text-destructive hover:bg-destructive/10 hover:text-destructive"
        aria-label="Dismiss error"
      >
        <HugeiconsIcon icon={Cancel01Icon} size={14} />
      </Button>
    </div>
  );
}

export function ZoneDetail({ zone }: { zone: ZoneState }) {
  const [showEq, setShowEq] = useState(false);
  const t = useTranslations();

  // Cover URL for ambient background glow — rendered at the ZoneDetail level
  // so it is structurally behind ALL interactive children and can never
  // intercept pointer events regardless of browser stacking quirks.
  const glowUrl = zone.source !== "idle" ? (zone.track?.cover_url ?? null) : null;

  return (
    <div className="relative flex flex-1 flex-col overflow-y-auto scrollbar-thin select-none @container">
      {/* Ambient cover glow */}
      {glowUrl && (
        <div className="absolute inset-0 pointer-events-none select-none isolate z-0" aria-hidden="true">
          <img
            src={glowUrl}
            alt=""
            className="absolute -top-12 -left-12 size-96 object-cover blur-3xl opacity-15 scale-125 -z-10"
          />
        </div>
      )}
      
      <div className="w-full max-w-[calc(100%-1.5rem)] mx-auto @sm:max-w-[680px] space-y-4 px-3 py-4 @sm:px-5 @sm:py-5 relative z-10 flex-1 flex flex-col justify-between">
        <div className="space-y-3">
          <ZoneHeader zone={zone} />
          <PlaybackErrorBanner zone={zone} />
          
          {/* Horizontal split layout if container width >= @sm (384px), else vertical stacked for mobile */}
          <div className="flex flex-col @sm:flex-row @sm:gap-6 @sm:items-start justify-center">
            {/* Cover Art */}
            <div className="w-full max-w-[280px] @sm:max-w-none @sm:w-44 @md:w-48 @lg:w-56 aspect-square mx-auto @sm:mx-0 shrink-0 shadow-lg rounded-2xl overflow-hidden transition-transform duration-500 hover:scale-[1.02] border border-white/5 bg-muted">
              <NowPlaying zone={zone} />
            </div>

            {/* Controls & Metadata */}
            <div className="flex flex-col justify-between flex-1 min-w-0 @sm:max-w-[360px] @md:max-w-[380px] @lg:max-w-[400px] space-y-3.5 @sm:space-y-0 @sm:h-44 @md:h-48 @lg:h-56 mt-4 @sm:mt-0">
              <TrackInfo zone={zone} forceCenter={false} />
              <SeekBar zone={zone} />
              <div className="flex items-center justify-center gap-3 relative w-full">
                <div className="mx-auto"><TransportControls zone={zone} /></div>
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => setShowEq(true)}
                  className={`absolute right-0 text-xs px-2.5 h-8 rounded-full bg-secondary/50 border border-border/20 ${zone.eqEnabled ? "text-orange-500 font-bold border-orange-500/30" : ""}`}
                  aria-label={t("eq.title", { zone: zone.name })}
                >
                  EQ
                </Button>
              </div>
              <ShuffleRepeat zone={zone} />
              <VolumeSlider
                volume={zone.volume}
                muted={zone.muted}
                onVolumeChange={(v) => api.zones.setVolume(zone.index, v).catch(logApiError)}
                onMuteToggle={() => api.zones.toggleMute(zone.index).catch(logApiError)}
                onUnmute={() => api.zones.setMute(zone.index, false).catch(logApiError)}
              />
            </div>
          </div>
        </div>

        {/* Playlists & Client sections */}
        <div className="border-t border-border/30 pt-4 space-y-4">
          <ClientList zone={zone} />
          <PlaylistBrowser zone={zone} />
        </div>
      </div>
      {showEq && <EqOverlay zoneId={zone.index} label={zone.name} onClose={(enabled) => { setShowEq(false); useAppStore.getState().updateZoneEq(zone.index, enabled); }} />}
    </div>
  );
}
