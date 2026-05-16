"use client";

import { useState } from "react";
import type { ZoneState } from "@/stores/useAppStore";

export function NowPlaying({ zone }: { zone: ZoneState }) {
  const track = zone.track;
  const isIdle = zone.source === "idle" || !track;
  const coverUrl = track?.cover_url || null;
  const [coverError, setCoverError] = useState(false);
  const [lastCover, setLastCover] = useState(coverUrl);

  // Reset error state when cover URL changes
  if (coverUrl !== lastCover) {
    setCoverError(false);
    setLastCover(coverUrl);
  }

  if (isIdle || !coverUrl) {
    return (
      <div className="relative w-full aspect-square rounded-2xl sm:rounded-xl overflow-hidden shadow-lg">
        <div className="flex flex-col items-center justify-center size-full bg-gradient-to-br from-muted to-muted/60">
          <div className="animate-pulse-slow">
            <img src="/assets/snapdog-icon.svg" alt="SnapDog" className="size-48 sm:size-32 opacity-30" />
          </div>
          <span className="text-xs font-medium text-muted-foreground/60 tracking-wider uppercase mt-2">{zone.name}</span>
        </div>
      </div>
    );
  }

  return (
    <div className="relative w-full aspect-square">
      {/* Color glow — blurred cover bleeds outside the container */}
      {!coverError && (
        <img
          key={`glow-${coverUrl}`}
          src={coverUrl}
          alt=""
          className="absolute -inset-4 w-[calc(100%+2rem)] h-[calc(100%+2rem)] object-cover blur-3xl opacity-30 scale-110 pointer-events-none transition-opacity duration-700"
        />
      )}
      {/* Main cover */}
      <div className="relative w-full h-full rounded-2xl sm:rounded-xl overflow-hidden bg-muted shadow-lg">
        {coverError ? (
          <div className="flex flex-col items-center justify-center size-full bg-gradient-to-br from-muted to-muted/60">
            <img src="/assets/snapdog-icon.svg" alt="SnapDog" className="size-48 sm:size-32 opacity-30" />
            <span className="text-xs font-medium text-muted-foreground/60 tracking-wider uppercase mt-2">{zone.name}</span>
          </div>
        ) : (
          <img
            key={`fg-${coverUrl}`}
            src={coverUrl}
            alt={`${track.title} cover`}
            className="w-full h-full object-contain animate-fade-in"
            onError={() => setCoverError(true)}
          />
        )}
      </div>
    </div>
  );
}
