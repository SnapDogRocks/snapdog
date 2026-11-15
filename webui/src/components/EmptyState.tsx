"use client";
import { useState, useEffect } from "react";
import { useTranslations } from "next-intl";
import { api } from "@/lib/api";
import { logApiError } from "@/lib/log-api-error";
export function EmptyState() {
  const t = useTranslations("empty");
  const [progMode, setProgMode] = useState(false);
  const [knxAvailable, setKnxAvailable] = useState(true);

  useEffect(() => {
    api.knx.getProgrammingMode()
      .then(setProgMode)
      .catch(() => { setKnxAvailable(false); });
  }, []);

  const toggleProg = () => {
    const next = !progMode;
    api.knx.setProgrammingMode(next)
      .then(() => { setProgMode(next); })
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
              aria-label={t("programmingMode")}
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
