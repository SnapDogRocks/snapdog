import { HugeiconsIcon } from "@hugeicons/react";
import { ShuffleIcon, RepeatIcon, RepeatOneIcon } from "@hugeicons/core-free-icons";
import { useTranslations } from "next-intl";
import { Button } from "@/components/ui/button";
import { api } from "@/lib/api";
import { logApiError } from "@/lib/log-api-error";
import type { ZoneState } from "@/stores/useAppStore";

interface ShuffleRepeatProps {
  zone: ZoneState;
}

export function ShuffleRepeat({ zone }: ShuffleRepeatProps) {
  const t = useTranslations("shuffle");
  const enabled = zone.source === "subsonic_playlist";
  const isRepeatPlaylist = zone.repeat === "playlist";
  const isRepeatTrack = zone.repeat === "track";
  const isRepeatAny = zone.repeat !== "off";

  return (
    <div className="flex items-center justify-center gap-1">
      <Button variant="ghost" size="icon" disabled={!enabled} onClick={() => api.zones.toggleShuffle(zone.index).catch(logApiError)} className={`size-8 rounded-full ${zone.shuffle ? "text-primary" : "text-muted-foreground"}`} aria-label={zone.shuffle ? t("on") : t("off")} aria-pressed={zone.shuffle}>
        <HugeiconsIcon icon={ShuffleIcon} size={16} />
      </Button>
      <Button variant="ghost" size="icon" disabled={!enabled} onClick={() => api.zones.toggleRepeat(zone.index).catch(logApiError)} className={`size-8 rounded-full ${isRepeatPlaylist ? "text-primary" : isRepeatTrack ? "text-primary" : "text-muted-foreground"}`} aria-label={isRepeatAny ? t("repeatOn") : t("repeatOff")} aria-pressed={isRepeatAny}>
        <HugeiconsIcon icon={isRepeatTrack ? RepeatOneIcon : RepeatIcon} size={16} />
      </Button>
    </div>
  );
}
