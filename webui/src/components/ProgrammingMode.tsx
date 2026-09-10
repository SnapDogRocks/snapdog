"use client";

import { useState, useEffect, useCallback } from "react";
import { useTranslations } from "next-intl";
import { api } from "@/lib/api";
import { logApiError } from "@/lib/log-api-error";
import { Button } from "@/components/ui/button";
import { useFocusTrap } from "@/hooks/useFocusTrap";

/**
 * Dezenter programming mode toggle in the toolbar.
 * Shows a small indicator that glows red when active.
 * Both enabling and disabling require confirmation.
 */
export function ProgrammingMode() {
  const t = useTranslations("knx");
  const [active, setActive] = useState(false);
  const [confirming, setConfirming] = useState<"on" | "off" | null>(null);
  const [available, setAvailable] = useState(true);

  // Poll current state on mount
  useEffect(() => {
    api.knx.getProgrammingMode()
      .then(setActive)
      .catch(() => { setAvailable(false); });
  }, []);

  const toggle = useCallback(() => {
    setConfirming(active ? "off" : "on");
  }, [active]);

  const confirm = useCallback(async () => {
    const newState = confirming === "on";
    try {
      await api.knx.setProgrammingMode(newState);
      setActive(newState);
    } catch (e) {
      logApiError(e);
    }
    setConfirming(null);
  }, [confirming]);

  const cancel = useCallback(() => { setConfirming(null); }, []);
  const trapRef = useFocusTrap<HTMLDivElement>();

  if (!available) return null;

  return (
    <>
      <Button
        variant="ghost"
        size="icon"
        onClick={toggle}
        className={`size-7 rounded-full transition-colors ${
          active
            ? "text-red-500 hover:text-red-600"
            : "text-muted-foreground/30 hover:text-muted-foreground/50"
        }`}
        aria-label={t(active ? "progModeOff" : "progModeOn")}
        title={t(active ? "progActive" : "progInactive")}
      >
        <span
          className={`size-2 rounded-full ${
            active ? "bg-red-500 animate-pulse" : "bg-current"
          }`}
          aria-hidden="true"
        />
      </Button>

      {/* Confirmation dialog */}
      {confirming && (
        <div
          className="fixed inset-0 z-50 flex items-center justify-center bg-black/50"
          role="dialog"
          aria-modal="true"
          onClick={cancel}
          onKeyDown={(e) => { if (e.key === "Escape") cancel(); }}
        >
          <div ref={trapRef} className="bg-card border border-border rounded-lg shadow-lg p-6 max-w-sm mx-4 space-y-4" onClick={(e) => { e.stopPropagation(); }}>
            <div className="flex items-start gap-3">
              {confirming === "on" && (
                <span className="text-amber-500 text-lg shrink-0" aria-hidden="true">⚠</span>
              )}
              <div className="space-y-2">
                <h2 className="text-sm font-semibold">
                  {t(confirming === "on" ? "confirmOn" : "confirmOff")}
                </h2>
                <p className="text-xs text-muted-foreground">
                  {t(confirming === "on" ? "confirmOnDesc" : "confirmOffDesc")}
                </p>
              </div>
            </div>
            <div className="flex justify-end gap-2">
              <Button variant="ghost" size="sm" onClick={cancel}>
                {t("cancel")}
              </Button>
              <Button
                variant={confirming === "on" ? "destructive" : "default"}
                size="sm"
                onClick={() => { void confirm(); }}
              >
                {t(confirming === "on" ? "enable" : "disable")}
              </Button>
            </div>
          </div>
        </div>
      )}
    </>
  );
}
