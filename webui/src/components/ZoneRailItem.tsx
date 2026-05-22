"use client";

import { useState } from "react";
import { useTranslations } from "next-intl";
import { useClientDrop } from "@/hooks/useClientDrop";
import type { ZoneState } from "@/stores/useAppStore";
import { Marquee } from "@/components/Marquee";

interface ZoneRailItemProps {
  zone: ZoneState;
  selected: boolean;
  onSelect: () => void;
}

export function ZoneRailItem({ zone, selected, onSelect }: ZoneRailItemProps) {
  const [imgError, setImgError] = useState(false);
  const { dragOver, dragHandlers } = useClientDrop(zone.index);
  const t = useTranslations();
  const isPlaying = zone.playback === "playing";
  const hasCover = zone.track?.cover_url && zone.source !== "idle" && !imgError;
  return (
    <button
      onClick={onSelect}
      {...dragHandlers}
      aria-current={selected ? "true" : undefined}
      className={`w-full flex items-center gap-3 px-3 py-3 rounded-lg text-left transition-all ${
        dragOver
          ? "bg-primary/20 ring-2 ring-primary"
          : selected
            ? "bg-primary/10 text-primary shadow-[0_0_12px_rgba(225,136,46,0.15)]"
            : "hover:bg-muted text-foreground"
      }`}
    >
      {/* Cover thumbnail or zone icon */}
      <div className="relative size-10 rounded-md bg-muted flex items-center justify-center overflow-hidden shrink-0">
        {hasCover ? (
          <img
            src={zone.track!.cover_url!}
            alt=""
            className="size-full object-cover"
            onError={() => setImgError(true)}
          />
        ) : (
          <span className="text-lg">{zone.icon || "🔊"}</span>
        )}
        {isPlaying && (
          <>
            <div className="absolute bottom-0.5 right-0.5 size-2 rounded-full bg-primary animate-pulse" />
            <span className="sr-only">{t("zone.playing")}</span>
          </>
        )}
      </div>
      <div className="min-w-0 flex-1">
        <div className="text-sm font-medium truncate">{zone.name}</div>
        <Marquee className="text-xs text-muted-foreground">
          {zone.track && zone.source !== "idle"
            ? `${zone.track.artist} — ${zone.track.title}`
            : t("zone.idle")}
        </Marquee>
      </div>
      <div className="flex flex-col items-end gap-1.5 shrink-0">
        <div className="text-xs text-muted-foreground tabular-nums">{zone.volume}</div>
        {zone.eqEnabled && (
          <span className="text-[8px] font-bold text-primary bg-primary/10 px-1 py-0.5 rounded uppercase tracking-wider scale-90" aria-label="EQ Active">
            EQ
          </span>
        )}
      </div>
    </button>
  );
}
