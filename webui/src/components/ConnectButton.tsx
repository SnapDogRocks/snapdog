"use client";

import { useState, useCallback } from "react";
import { QRCodeSVG } from "qrcode.react";
import { useFocusTrap } from "@/hooks/useFocusTrap";
import { motion, AnimatePresence, useMotionValue, useTransform } from "framer-motion";
import { HugeiconsIcon } from "@hugeicons/react";
import { Cancel01Icon, Share08Icon } from "@hugeicons/core-free-icons";
import { useTranslations } from "next-intl";

export function ConnectButton() {
  const [open, setOpen] = useState(false);
  const t = useTranslations("connect");

  return (
    <>
      <button
        onClick={() => setOpen(true)}
        className="p-1 rounded-md text-muted-foreground hover:text-foreground hover:bg-muted/55 transition-colors cursor-pointer"
        aria-label={t("title")}
      >
        <ShareIcon size={16} />
      </button>
      <AnimatePresence>
        {open && <ConnectOverlay onClose={() => setOpen(false)} />}
      </AnimatePresence>
    </>
  );
}

function ConnectOverlay({ onClose }: { onClose: () => void }) {
  const [baseUrl] = useState(() =>
    typeof window !== "undefined" ? window.location.origin : ""
  );
  const [apiKey] = useState(() =>
    typeof window !== "undefined" ? sessionStorage.getItem("snapdog_api_key") : null
  );
  const [copied, setCopied] = useState(false);
  const trapRef = useFocusTrap<HTMLDivElement>();
  const t = useTranslations("connect");

  const deepLink = apiKey
    ? `snapdog://connect?url=${encodeURIComponent(baseUrl)}&token=${encodeURIComponent(apiKey)}`
    : `snapdog://connect?url=${encodeURIComponent(baseUrl)}`;

  const shareUrl = apiKey ? `${baseUrl}/?auth=${apiKey}` : baseUrl;

  const handleShare = useCallback(async () => {
    if (navigator.share) {
      try {
        await navigator.share({ title: "SnapDog", url: shareUrl });
      } catch {
        // User cancelled
      }
    } else {
      await navigator.clipboard.writeText(shareUrl);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    }
  }, [shareUrl]);

  // Motion values for swipe/drag close physics
  const y = useMotionValue(0);
  const backdropOpacity = useTransform(y, [0, 250], [0.7, 0], { clamp: true });
  const blurAmount = useTransform(y, [0, 250], [12, 0], { clamp: true });
  const backdropFilter = useTransform(blurAmount, (v) => `blur(${v}px)`);
  const cardScale = useTransform(y, [0, 250], [1, 0.95], { clamp: true });

  return (
    <div
      className="fixed inset-0 z-50 flex items-end sm:items-center justify-center overflow-hidden"
      role="dialog"
      aria-modal="true"
      aria-label={t("dialogLabel")}
      onKeyDown={(e) => { if (e.key === "Escape") onClose(); }}
    >
      {/* Backdrop */}
      <motion.div
        initial={{ opacity: 0 }}
        animate={{ opacity: 1 }}
        exit={{ opacity: 0 }}
        style={{ opacity: backdropOpacity, backdropFilter, WebkitBackdropFilter: backdropFilter }}
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
          if (info.offset.y > 120 || info.velocity.y > 500) onClose();
        }}
        initial={{ y: 600, opacity: 0 }}
        animate={{ y: 0, opacity: 1 }}
        exit={{ y: 600, opacity: 0 }}
        transition={{ type: "spring", damping: 30, stiffness: 300 }}
        className="relative z-10 w-full max-w-none sm:max-w-sm mx-0 sm:mx-4 rounded-t-3xl sm:rounded-2xl border-t border-x sm:border border-border bg-card p-5 sm:p-6 pb-6 shadow-2xl flex flex-col items-center gap-4 text-center touch-none select-none cursor-default flex-shrink-0"
      >
        {/* Drag handle */}
        <div className="w-12 h-1 rounded-full bg-muted-foreground/20 mx-auto cursor-grab active:cursor-grabbing shrink-0 hover:bg-muted-foreground/45 transition-colors" />

        {/* Close button */}
        <motion.button
          onClick={onClose}
          whileHover={{ scale: 1.1, rotate: 90 }}
          whileTap={{ scale: 0.95 }}
          className="absolute top-4 right-4 p-1.5 rounded-full text-muted-foreground hover:text-foreground hover:bg-muted/80 transition-colors cursor-pointer shrink-0 z-20"
          aria-label={t("closeLabel")}
        >
          <HugeiconsIcon icon={Cancel01Icon} size={16} />
        </motion.button>

        {/* Title */}
        <div className="flex flex-col items-center gap-1 mt-1">
          <h2 className="text-xl sm:text-2xl font-bold tracking-tight">{t("title")}</h2>
          <p className="text-xs text-muted-foreground leading-relaxed px-4">
            {t("description")}
          </p>
        </div>

        {/* QR Code */}
        <div className="p-4 bg-white rounded-2xl shadow-inner">
          <QRCodeSVG
            value={deepLink}
            size={180}
            level="M"
            imageSettings={{
              src: "/assets/snapdog-icon.svg",
              height: 36,
              width: 36,
              excavate: true,
            }}
          />
        </div>

        {/* URL display */}
        <div className="w-full px-3 py-2 rounded-lg bg-muted/15 dark:bg-muted/5 border border-border/30 font-mono text-[11px] text-muted-foreground truncate">
          {baseUrl}
        </div>

        {/* Security note */}
        {apiKey && (
          <p className="text-[10px] text-muted-foreground/60 px-4 leading-relaxed">
            {t("securityNote")}
          </p>
        )}

        {/* Share button */}
        <motion.button
          onClick={handleShare}
          whileHover={{ scale: 1.01 }}
          whileTap={{ scale: 0.98 }}
          className="w-full py-2.5 bg-primary text-primary-foreground font-semibold rounded-xl hover:bg-primary/95 active:scale-[0.98] transition-all duration-150 shadow-md shadow-primary/10 text-sm cursor-pointer flex items-center justify-center gap-2"
        >
          <HugeiconsIcon icon={Share08Icon} size={16} />
          {copied ? t("copied") : t("share")}
        </motion.button>
      </motion.div>
    </div>
  );
}

function ShareIcon({ size = 16 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={1.5} strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <path d="M12 3v12" />
      <path d="m8 7 4-4 4 4" />
      <path d="M20 21H4a1 1 0 0 1-1-1v-9a1 1 0 0 1 1-1h3m10 0h3a1 1 0 0 1 1 1v9a1 1 0 0 1-1 1" />
    </svg>
  );
}
