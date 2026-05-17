"use client";

import { useState, useEffect, useRef, useCallback } from "react";
import { useTranslations } from "next-intl";
import { useI18n } from "@/i18n/provider";
import { locales, type Locale } from "@/i18n/config";

const labels: Record<Locale, string> = {
  en: "English",
  de: "Deutsch",
  fr: "Français",
  es: "Español",
  nl: "Nederlands",
};

function GlobeIcon({ size = 16 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={1.5} strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <circle cx="12" cy="12" r="10" />
      <ellipse cx="12" cy="12" rx="4" ry="10" />
      <path d="M2 12h20" />
    </svg>
  );
}

export function LocalePicker() {
  const t = useTranslations("app");
  const { locale, setLocale } = useI18n();
  const [open, setOpen] = useState(false);
  const containerRef = useRef<HTMLDivElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const [focusIdx, setFocusIdx] = useState(-1);

  const close = useCallback(() => { setOpen(false); setFocusIdx(-1); }, []);
  const openMenu = useCallback(() => {
    const idx = locales.indexOf(locale);
    setFocusIdx(idx >= 0 ? idx : 0);
    setOpen(true);
  }, [locale]);

  const select = useCallback((l: Locale) => {
    setLocale(l);
    close();
  }, [setLocale, close]);

  // Focus the active button when focusIdx changes
  useEffect(() => {
    if (!open || focusIdx < 0) return;
    const buttons = menuRef.current?.querySelectorAll<HTMLButtonElement>("[role=menuitem]");
    buttons?.[focusIdx]?.focus();
  }, [open, focusIdx]);

  // Outside click + keyboard
  useEffect(() => {
    if (!open) return;
    const onClickOutside = (e: MouseEvent) => {
      if (containerRef.current && !containerRef.current.contains(e.target as Node)) close();
    };
    const onKey = (e: KeyboardEvent) => {
      switch (e.key) {
        case "Escape":
          close();
          break;
        case "ArrowDown":
          e.preventDefault();
          setFocusIdx((i) => (i + 1) % locales.length);
          break;
        case "ArrowUp":
          e.preventDefault();
          setFocusIdx((i) => (i - 1 + locales.length) % locales.length);
          break;
        case "Tab":
          close();
          break;
      }
    };
    document.addEventListener("mousedown", onClickOutside);
    document.addEventListener("keydown", onKey);
    return () => { document.removeEventListener("mousedown", onClickOutside); document.removeEventListener("keydown", onKey); };
  }, [open, close]);

  return (
    <div className="relative" ref={containerRef}>
      <button
        onClick={() => { if (open) { close(); } else { openMenu(); } }}
        className="p-1 rounded-md text-muted-foreground hover:text-foreground hover:bg-muted/50 transition-colors"
        aria-label={t("language")}
        aria-expanded={open}
        aria-haspopup="menu"
      >
        <GlobeIcon size={16} />
      </button>
      {open && (
        <div
          ref={menuRef}
          role="menu"
          aria-label={t("language")}
          className="absolute right-0 top-full mt-1.5 z-50 min-w-[9rem] rounded-xl bg-popover/95 backdrop-blur-xl border border-border/50 shadow-lg py-1"
        >
          {locales.map((l) => (
            <button
              key={l}
              onClick={() => select(l)}
              role="menuitem"
              tabIndex={-1}
              className={`w-full text-left px-3 py-1.5 text-sm transition-colors flex items-center justify-between ${
                l === locale
                  ? "text-primary font-medium"
                  : "text-foreground hover:bg-muted/50"
              }`}
            >
              {labels[l]}
              {l === locale && <span className="text-primary text-xs">✓</span>}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
