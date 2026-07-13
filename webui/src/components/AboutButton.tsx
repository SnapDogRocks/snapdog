"use client";

import { useState, useEffect } from "react";
import { useFocusTrap } from "@/hooks/useFocusTrap";
import { api, type KnxProductInfo } from "@/lib/api";
import { motion, AnimatePresence, useMotionValue, useTransform } from "framer-motion";
import { HugeiconsIcon } from "@hugeicons/react";
import { Cancel01Icon, Download04Icon } from "@hugeicons/core-free-icons";
import { useTranslations } from "next-intl";

export function AboutButton() {
  const [open, setOpen] = useState(false);
  const t = useTranslations("about");

  return (
    <>
      <button
        onClick={() => setOpen(true)}
        className="p-1 rounded-md text-muted-foreground hover:text-foreground hover:bg-muted/55 transition-colors cursor-pointer"
        aria-label={t("title")}
      >
        <InfoIcon size={16} />
      </button>
      <AnimatePresence>
        {open && <AboutOverlay onClose={() => setOpen(false)} />}
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
  const trapRef = useFocusTrap<HTMLDivElement>();
  const t = useTranslations("about");

  useEffect(() => {
    api.system.version().then((v) => { setVersion(v.version); setServerName(v.name); }).catch(() => {});
    api.knx.getProgrammingMode().then((mode) => {
      setProgMode(mode);
      setKnxAvailable(true);
    }).catch(() => {});
    api.knx.getProductInfo().then(setKnxProduct).catch(() => {});
    if (typeof window !== "undefined") {
      setTimeout(() => {
        setHostname(window.location.hostname);
      }, 0);
    }
  }, []);

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
        className="relative z-10 w-full max-w-none sm:max-w-sm mx-0 sm:mx-4 rounded-t-3xl sm:rounded-2xl border-t border-x sm:border border-border bg-card p-5 sm:p-6 pb-6 shadow-2xl flex flex-col items-center gap-4 text-center touch-none select-none cursor-default flex-shrink-0"
      >
        {/* Drag handle line indicator */}
        <div className="w-12 h-1 rounded-full bg-muted-foreground/20 mx-auto cursor-grab active:cursor-grabbing shrink-0 hover:bg-muted-foreground/45 transition-colors" />

        {/* Top-right floating close button */}
        <motion.button
          onClick={onClose}
          whileHover={{ scale: 1.1, rotate: 90 }}
          whileTap={{ scale: 0.95 }}
          className="absolute top-4 right-4 p-1.5 rounded-full text-muted-foreground hover:text-foreground hover:bg-muted/80 transition-colors cursor-pointer shrink-0 z-20"
          aria-label={t("closeLabel")}
        >
          <HugeiconsIcon icon={Cancel01Icon} size={16} />
        </motion.button>

        {/* Host bar */}
        <div className="w-full flex items-center justify-center px-1 text-xs font-mono text-muted-foreground/70 border-b border-border/30 pb-2.5 flex-shrink-0">
          <span>{serverName !== "SnapDog" ? serverName : hostname || "..."}</span>
        </div>

        {/* Header Group (Logo, Headings, Soundwave) */}
        <div className="flex flex-col items-center gap-2 mt-0.5 flex-shrink-0">
          <img
            src="/assets/snapdog-icon.svg"
            alt="SnapDog Logo"
            className="size-12 sm:size-16 mx-auto opacity-95 animate-pulse-slow"
          />
          <h2 className="text-xl sm:text-2xl font-bold tracking-tight mt-0.5">SnapDog</h2>
          <p className="text-xs text-muted-foreground leading-relaxed px-4">
            {t("description")}
          </p>

          {/* Retro digital soundwave visualizer */}
          <div className="flex items-center justify-center gap-0.5 h-4 mt-2">
            {[...Array(9)].map((_, i) => (
              <motion.div
                key={i}
                animate={{
                  height: [4, 16, 4],
                }}
                transition={{
                  duration: 1.0 + (i % 3) * 0.25,
                  repeat: Infinity,
                  ease: "easeInOut",
                  delay: i * 0.08,
                }}
                className="w-[2px] rounded-full bg-primary/60"
              />
            ))}
          </div>
        </div>

        {/* Monospace Tech Spec Grid */}
        <div className="grid grid-cols-2 gap-2 w-full font-mono text-left text-xs pt-1 flex-shrink-0">
          <div className="flex flex-col py-2 px-3 rounded-lg bg-muted/15 dark:bg-muted/5 border border-border/30">
            <span className="text-[8px] sm:text-[9px] uppercase font-semibold text-muted-foreground/65 tracking-wider">{t("modelLabel")}</span>
            <span className="font-semibold text-foreground mt-0.5 tracking-tight text-xs sm:text-sm">{t("modelValue")}</span>
          </div>

          <div className="flex flex-col py-2 px-3 rounded-lg bg-muted/15 dark:bg-muted/5 border border-border/30 translate-x-[40px]">
            <span className="text-[8px] sm:text-[9px] uppercase font-semibold text-muted-foreground/65 tracking-wider">{t("versionLabel")}</span>
            <span className="font-semibold text-foreground mt-0.5 tabular-nums tracking-tight text-xs sm:text-sm">
              {version ? `v${version}` : "v0.11.4"}
            </span>
          </div>

          <a
            href="https://github.com/SnapDogRocks/snapdog"
            target="_blank"
            rel="noopener noreferrer"
            className="flex flex-col py-2 px-3 rounded-lg bg-muted/15 dark:bg-muted/5 border border-border/30 hover:bg-muted/30 dark:hover:bg-muted/15 hover:border-border/60 transition-all duration-200 group cursor-pointer"
          >
            <span className="text-[8px] sm:text-[9px] uppercase font-semibold text-muted-foreground/65 tracking-wider flex items-center gap-1">
              {t("sourceLabel")} <GitHubIcon size={8} />
            </span>
            <span className="font-semibold text-primary mt-0.5 group-hover:underline tracking-tight text-xs sm:text-sm whitespace-nowrap">
              {t("sourceValue")}
            </span>
          </a>

          <a
            href="https://www.gnu.org/licenses/gpl-3.0.html"
            target="_blank"
            rel="noopener noreferrer"
            className="flex flex-col py-2 px-3 rounded-lg bg-muted/15 dark:bg-muted/5 border border-border/30 hover:bg-muted/30 dark:hover:bg-muted/15 hover:border-border/60 transition-all duration-200 group cursor-pointer translate-x-[40px]"
          >
            <span className="text-[8px] sm:text-[9px] uppercase font-semibold text-muted-foreground/65 tracking-wider">{t("licenseLabel")}</span>
            <span className="font-semibold text-primary mt-0.5 group-hover:underline tracking-tight text-xs sm:text-sm">
              {t("licenseValue")}
            </span>
          </a>
        </div>

        {/* KNX Programming Mode — only visible when KNX is configured */}
        {knxAvailable && (
          <div className="w-full flex items-center justify-between py-2.5 px-3 rounded-lg bg-muted/15 dark:bg-muted/5 border border-border/30 font-mono">
            <div className="flex flex-col">
              <span className="text-[8px] sm:text-[9px] uppercase font-semibold text-muted-foreground/65 tracking-wider">KNX</span>
              <span className="font-semibold text-foreground text-xs sm:text-sm tracking-tight">Programming Mode</span>
            </div>
            <button
              onClick={() => {
                const next = !progMode;
                api.knx.setProgrammingMode(next).then(() => setProgMode(next)).catch(() => {});
              }}
              className={`relative w-11 h-6 rounded-full transition-colors ${progMode ? "bg-red-500" : "bg-muted"}`}
              role="switch"
              aria-checked={progMode}
              aria-label="KNX Programming Mode"
            >
              <span className={`absolute top-0.5 left-0.5 size-5 rounded-full bg-white shadow transition-transform ${progMode ? "translate-x-5" : ""}`} />
            </button>
          </div>
        )}

        {/* KNX ETS product database (.knxprod) download — for importing SnapDog into ETS */}
        <button
          onClick={() => { api.knx.downloadKnxprod().catch(() => {}); }}
          className="w-full flex items-center justify-between py-2.5 px-3 rounded-lg bg-muted/15 dark:bg-muted/5 border border-border/30 hover:bg-muted/30 dark:hover:bg-muted/15 hover:border-border/60 transition-all duration-200 group cursor-pointer text-left"
        >
          <div className="flex flex-col min-w-0">
            <span className="text-[8px] sm:text-[9px] uppercase font-semibold text-muted-foreground/65 tracking-wider">KNX / ETS</span>
            <span className="font-semibold text-foreground text-xs sm:text-sm tracking-tight group-hover:text-primary transition-colors">
              {t("knxprodTitle")}
            </span>
            {knxProduct && (
              <span className="text-[9px] sm:text-[10px] font-mono text-muted-foreground/50 tracking-tight mt-0.5">
                v{knxProduct.app_version} · {knxProduct.application_number} · HW {knxProduct.hardware_version}
              </span>
            )}
          </div>
          <HugeiconsIcon icon={Download04Icon} className="size-5 text-muted-foreground/60 group-hover:text-primary transition-colors shrink-0" />
        </button>

        {/* Footer Group (Copyright & Done Button) */}
        <div className="w-full flex flex-col items-center gap-3 mt-1 flex-shrink-0">
          <div className="text-[10px] font-mono text-muted-foreground/45">
            © 2026 Fabian Schmieder
          </div>

          {/* Prominent main done button */}
          <motion.button
            onClick={onClose}
            whileHover={{ scale: 1.01 }}
            whileTap={{ scale: 0.98 }}
            className="w-full py-2.5 bg-primary text-primary-foreground font-semibold rounded-xl hover:bg-primary/95 active:scale-[0.98] transition-all duration-150 shadow-md shadow-primary/10 text-sm cursor-pointer"
          >
            {t("done")}
          </motion.button>
        </div>
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
