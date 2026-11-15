"use client";
import { useTranslations } from "next-intl";
import { HugeiconsIcon } from "@hugeicons/react";
import { AlertCircleIcon, Cancel01Icon } from "@hugeicons/core-free-icons";
import { useAppStore, type ZoneState } from "@/stores/useAppStore";
import { Button } from "@/components/ui/button";
export function PlaybackErrorBanner({ zone }: { zone: ZoneState }) {
  const t = useTranslations("current");
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
              {t("details")}
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
        onClick={() => { useAppStore.getState().setZoneError(zone.index, null); }}
        className="-mr-1 -mt-1 size-7 shrink-0 rounded-full text-destructive hover:bg-destructive/10 hover:text-destructive"
        aria-label={t("dismiss")}
      >
        <HugeiconsIcon icon={Cancel01Icon} size={14} />
      </Button>
    </div>
  );
}
