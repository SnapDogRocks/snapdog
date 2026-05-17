import { useState, useCallback } from "react";
import { locales, defaultLocale, type Locale } from "./config";

const STORAGE_KEY = "snapdog-locale";

function detect(): Locale {
  if (typeof window === "undefined") return defaultLocale;
  const stored = localStorage.getItem(STORAGE_KEY);
  if (stored && locales.includes(stored as Locale)) return stored as Locale;
  const nav = navigator.language.split("-")[0];
  if (locales.includes(nav as Locale)) return nav as Locale;
  return defaultLocale;
}

export function useLocaleState() {
  const [locale, setLocaleState] = useState<Locale>(() => detect());

  const setLocale = useCallback((l: Locale) => {
    localStorage.setItem(STORAGE_KEY, l);
    setLocaleState(l);
  }, []);

  return { locale, setLocale };
}
