"use client";

import { useState, useEffect } from "react";
import { useFocusTrap } from "@/hooks/useFocusTrap";
import { api, type KnxProductInfo } from "@/lib/api";
import type { UpdateStatus } from "@/lib/types";
import { motion, AnimatePresence, useMotionValue, useTransform } from "framer-motion";
import { HugeiconsIcon } from "@hugeicons/react";
import { ArrowUpRight01Icon, Cancel01Icon, Download04Icon } from "@hugeicons/core-free-icons";
import { useTranslations } from "next-intl";

const ignoreUnavailableOptionalEndpoint = () => undefined;

export function AboutButton() {
  const [open, setOpen] = useState(false);
  const t = useTranslations("about");

  return (
    <>
      <button
        onClick={() => { setOpen(true); }}
        className="p-1 rounded-md text-muted-foreground hover:text-foreground hover:bg-muted/55 transition-colors cursor-pointer"
        aria-label={t("title")}
      >
        <InfoIcon size={16} />
      </button>
      <AnimatePresence>
        {open && <AboutOverlay onClose={() => { setOpen(false); }} />}
      </AnimatePresence>
    </>
  );
}

function AboutOverlay({ onClose }: { onClose: () => void }) {
  const [version, setVersion] = useState<string | null>(null);
  const [serverName, setServerName] = useState<string>("SnapDog");
  const [hostname, setHostname] = useState<string>("");
  const [knxAvailable, setKnxAvailable] = useState(false);
  const [progMode, setProgMode] = useState(false);
  const [knxProduct, setKnxProduct] = useState<KnxProductInfo | null>(null);
  const [update, setUpdate] = useState<UpdateStatus>();
  const [updateBusy, setUpdateBusy] = useState(false);
  const trapRef = useFocusTrap<HTMLDivElement>();
  const t = useTranslations("about");
  // The updater reports its state as a token; a state this interface does not know shows as is.
  const updateStateLabel = (state: string) => {
    const key = `updateState.${state.replaceAll("-", "_")}`;
    return t.has(key) ? t(key) : state;
  };

  useEffect(() => {
    api.system
      .version()
      .then((v) => {
        setVersion(v.version);
        setServerName(v.name);
      })
      .catch(ignoreUnavailableOptionalEndpoint);
    api.knx
      .getProgrammingMode()
      .then((mode) => {
        setProgMode(mode);
        setKnxAvailable(true);
      })
      .catch(ignoreUnavailableOptionalEndpoint);
    api.knx.getProductInfo().then(setKnxProduct).catch(ignoreUnavailableOptionalEndpoint);
    api.system.updateStatus().then(setUpdate).catch(ignoreUnavailableOptionalEndpoint);
    if (typeof window !== "undefined") {
      setTimeout(() => {
        setHostname(window.location.hostname);
      }, 0);
    }
  }, []);

  useEffect(() => {
    if (!updateBusy) return;
    const timer = window.setInterval(() => {
      api.system
        .updateStatus()
        .then((value) => {
          setUpdate(value);
          if (
            ![
              "checking",
              "preparing",
              "updating",
              "verifying",
              "deploying",
              "waiting-for-health",
              "rolling-back",
            ].includes(value.state)
          ) {
            setUpdateBusy(false);
          }
        })
        .catch(ignoreUnavailableOptionalEndpoint);
    }, 2000);
    return () => {
      window.clearInterval(timer);
    };
  }, [updateBusy]);

  // Motion values for swipe/drag close physics (strictly numeric for interpolation safety)
  const y = useMotionValue(0);

  // Transform y distance into backdrop opacity (0.7 -> 0) and blur (12px -> 0px)
  const backdropOpacity = useTransform(y, [0, 250], [0.7, 0], { clamp: true });
  const blurAmount = useTransform(y, [0, 250], [12, 0], { clamp: true });
  const backdropFilter = useTransform(blurAmount, (v) => `blur(${v}px)`);

  // Transform y distance into minor scale change on card
  const cardScale = useTransform(y, [0, 250], [1, 0.95], { clamp: true });

  return (
    <div
      className="fixed inset-0 z-50 flex items-end sm:items-center justify-center overflow-hidden"
      role="dialog"
      aria-modal="true"
      aria-label={t("dialogLabel")}
      onKeyDown={(e) => {
        if (e.key === "Escape") onClose();
      }}
    >
      {/* Backdrop */}
      <motion.div
        initial={{ opacity: 0 }}
        animate={{ opacity: 1 }}
        exit={{ opacity: 0 }}
        style={{
          opacity: backdropOpacity,
          backdropFilter: backdropFilter,
          WebkitBackdropFilter: backdropFilter,
        }}
        transition={{ duration: 0.2 }}
        className="absolute inset-0 bg-background/80 cursor-pointer"
        onClick={onClose}
        role="presentation"
      />

      {/* Card */}
      <motion.div
        ref={trapRef}
        drag="y"
        dragConstraints={{ top: 0, bottom: 600 }}
        dragElastic={{ top: 0.05, bottom: 0.75 }}
        style={{ y, scale: cardScale }}
        onDragEnd={(_, info) => {
          if (info.offset.y > 120 || info.velocity.y > 500) {
            onClose();
          }
        }}
        initial={{ y: 600, opacity: 0 }}
        animate={{ y: 0, opacity: 1 }}
        exit={{ y: 600, opacity: 0 }}
        transition={{ type: "spring", damping: 30, stiffness: 300 }}
        className="relative z-10 w-full max-w-none sm:max-w-md mx-0 sm:mx-4 rounded-t-3xl sm:rounded-2xl border-t border-x sm:border border-border bg-card p-4 sm:p-5 pb-5 shadow-2xl flex flex-col gap-3 text-left touch-none select-none cursor-default flex-shrink-0"
      >
        {/* Drag handle: the way to close the sheet on a phone */}
        <div className="w-12 h-1 rounded-full bg-muted-foreground/20 mx-auto cursor-grab active:cursor-grabbing shrink-0 hover:bg-muted-foreground/45 transition-colors sm:hidden" />

        <motion.button
          onClick={onClose}
          whileHover={{ scale: 1.1, rotate: 90 }}
          whileTap={{ scale: 0.95 }}
          className="absolute top-3 right-3 p-1.5 rounded-full text-muted-foreground hover:text-foreground hover:bg-muted/80 transition-colors cursor-pointer shrink-0 z-20"
          aria-label={t("closeLabel")}
        >
          <HugeiconsIcon icon={Cancel01Icon} size={16} />
        </motion.button>

        {/* Header: logo, name, version, host and model in two lines */}
        <div className="flex items-center gap-3 pr-8">
          <img
            src="/assets/snapdog-icon.svg"
            alt=""
            className="size-12 shrink-0 opacity-95 animate-pulse-slow"
          />
          <div className="min-w-0 flex-1">
            <div className="flex items-center gap-2">
              <h2 className="text-lg font-bold tracking-tight">SnapDog</h2>
              {version && (
                <span className="rounded-full bg-primary/15 px-2 py-0.5 font-mono text-xs tabular-nums text-primary-ink">
                  v{version}
                </span>
              )}
            </div>
            <div className="truncate font-mono text-xs text-muted-foreground/70">
              {serverName !== "SnapDog" ? serverName : hostname || "…"} · {t("modelValue")}
            </div>
          </div>
        </div>

        <p className="text-xs text-muted-foreground leading-relaxed">{t("description")}</p>

        {update && (
          <div className="flex items-center justify-between gap-3 rounded-lg border border-border/30 bg-muted/15 px-3 py-2.5 dark:bg-muted/5">
            <div className="flex min-w-0 flex-col">
              <span className="text-[9px] font-semibold uppercase tracking-wider text-muted-foreground/65">{t("updateTitle")}</span>
              <span className="truncate text-xs font-semibold">
                {updateStateLabel(update.progress?.phase ?? update.state)}
                {update.availableVersion ? ` · ${update.availableVersion}` : ""}
              </span>
              {update.lastError && <span className="truncate text-[10px] text-destructive">{update.lastError}</span>}
            </div>
            <button
              disabled={updateBusy}
              onClick={() => {
                setUpdateBusy(true);
                const operation = update.updateAvailable ? api.system.applyUpdate() : api.system.checkUpdates();
                operation.then(setUpdate).catch(() => {
                  setUpdateBusy(false);
                });
              }}
              className="shrink-0 rounded-lg bg-primary px-3 py-1.5 text-xs font-semibold text-primary-foreground disabled:opacity-50 cursor-pointer"
            >
              {updateBusy ? "…" : update.updateAvailable ? t("updateApply") : t("updateCheck")}
            </button>
          </div>
        )}

        {/* KNX: the programming mode (only when KNX runs as a device) and the ETS product database */}
        <div className="divide-y divide-border/30 overflow-hidden rounded-lg border border-border/30 bg-muted/15 dark:bg-muted/5">
          {knxAvailable && (
            <div className="flex items-center justify-between gap-3 px-3 py-2.5">
              <div className="flex flex-col">
                <span className="text-[9px] font-semibold uppercase tracking-wider text-muted-foreground/65">{t("knxTitle")}</span>
                <span className="text-xs font-semibold sm:text-sm">{t("programmingMode")}</span>
              </div>
              <button
                onClick={() => {
                  const next = !progMode;
                  api.knx.setProgrammingMode(next).then(() => { setProgMode(next); }).catch(() => { /* leave progMode as-is on failure */ });
                }}
                className={`relative h-6 w-11 shrink-0 rounded-full transition-colors cursor-pointer ${progMode ? "bg-red-500" : "bg-muted"}`}
                role="switch"
                aria-checked={progMode}
                aria-label={t("programmingMode")}
              >
                <span className={`absolute left-0.5 top-0.5 size-5 rounded-full bg-white shadow transition-transform ${progMode ? "translate-x-5" : ""}`} />
              </button>
            </div>
          )}
          <button
            onClick={() => { api.knx.downloadKnxprod().catch(() => { /* download failed silently; nothing to recover here */ }); }}
            className="group flex w-full items-center justify-between gap-3 px-3 py-2.5 text-left transition-colors hover:bg-muted/30 dark:hover:bg-muted/15 cursor-pointer"
          >
            <div className="flex min-w-0 flex-col">
              <span className="text-xs font-semibold tracking-tight transition-colors group-hover:text-primary-ink sm:text-sm">
                {t("knxprodTitle")}
              </span>
              {knxProduct && (
                <span className="mt-0.5 font-mono text-[10px] tracking-tight text-muted-foreground/50">
                  v{knxProduct.app_version} · {knxProduct.application_number} · HW {knxProduct.hardware_version}
                </span>
              )}
            </div>
            <HugeiconsIcon icon={Download04Icon} className="size-5 shrink-0 text-muted-foreground/60 transition-colors group-hover:text-primary-ink" />
          </button>
        </div>

        <div className="flex flex-wrap justify-center gap-2">
          <a
            href="https://github.com/SnapDogRocks/snapdog"
            target="_blank"
            rel="noopener noreferrer"
            className="inline-flex items-center gap-1.5 rounded-lg border border-border/40 px-2.5 py-1.5 font-mono text-xs text-primary-ink transition-colors hover:bg-muted/30 hover:border-border/70 cursor-pointer"
          >
            <GitHubIcon size={12} />
            {t("sourceValue")}
            <HugeiconsIcon icon={ArrowUpRight01Icon} size={12} />
          </a>
          <a
            href="https://www.gnu.org/licenses/gpl-3.0.html"
            target="_blank"
            rel="noopener noreferrer"
            className="inline-flex items-center gap-1.5 rounded-lg border border-border/40 px-2.5 py-1.5 font-mono text-xs text-primary-ink transition-colors hover:bg-muted/30 hover:border-border/70 cursor-pointer"
          >
            {t("licenseValue")}
            <HugeiconsIcon icon={ArrowUpRight01Icon} size={12} />
          </a>
        </div>

        <div className="text-center font-mono text-[10px] text-muted-foreground/45">© 2026 Fabian Schmieder</div>
      </motion.div>
    </div>
  );
}

function InfoIcon({ size = 16 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={1.5} strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <circle cx="12" cy="12" r="10" />
      <path d="M12 16v-4" />
      <path d="M12 8h.01" />
    </svg>
  );
}

function GitHubIcon({ size = 16 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">
      <path d="M12 0C5.37 0 0 5.37 0 12c0 5.31 3.435 9.795 8.205 11.385.6.105.825-.255.825-.57 0-.285-.015-1.23-.015-2.235-3.015.555-3.795-.735-4.035-1.41-.135-.345-.72-1.41-1.23-1.695-.42-.225-1.02-.78-.015-.795.945-.015 1.62.87 1.845 1.23 1.08 1.815 2.805 1.305 3.495.99.105-.78.42-1.305.765-1.605-2.67-.3-5.46-1.335-5.46-5.925 0-1.305.465-2.385 1.23-3.225-.12-.3-.54-1.53.12-3.18 0 0 1.005-.315 3.3 1.23.96-.27 1.98-.405 3-.405s2.04.135 3 .405c2.295-1.56 3.3-1.23 3.3-1.23.66 1.65.24 2.88.12 3.18.765.84 1.23 1.905 1.23 3.225 0 4.605-2.805 5.625-5.475 5.925.435.375.81 1.095.81 2.22 0 1.605-.015 2.895-.015 3.3 0 .315.225.69.825.57A12.02 12.02 0 0024 12c0-6.63-5.37-12-12-12z" />
    </svg>
  );
}
