"use client";

import { useState, useEffect, type ReactNode } from "react";
import { NextIntlClientProvider } from "next-intl";
import { useLocaleState } from "./useLocaleState";
import type { Locale } from "./config";

const messageImports: Record<Locale, () => Promise<{ default: Record<string, unknown> }>> = {
  en: () => import("../../messages/en.json"),
  de: () => import("../../messages/de.json"),
  fr: () => import("../../messages/fr.json"),
  es: () => import("../../messages/es.json"),
  it: () => import("../../messages/it.json"),
  nl: () => import("../../messages/nl.json"),
};

export function I18nProvider({ children }: { children: ReactNode }) {
  const { locale, setLocale } = useLocaleState();
  const [messages, setMessages] = useState<Record<string, unknown> | null>(null);

  useEffect(() => {
    messageImports[locale]().then((m) => setMessages(m.default));
    document.documentElement.lang = locale;
  }, [locale]);

  if (!messages) return null;

  return (
    <I18nContext.Provider value={{ locale, setLocale }}>
      <NextIntlClientProvider locale={locale} messages={messages}>
        {children}
      </NextIntlClientProvider>
    </I18nContext.Provider>
  );
}

// Context for locale picker
import { createContext, useContext } from "react";

interface I18nContextValue {
  locale: Locale;
  setLocale: (l: Locale) => void;
}

const I18nContext = createContext<I18nContextValue>({
  locale: "en",
  setLocale: () => {},
});

export const useI18n = () => useContext(I18nContext);
