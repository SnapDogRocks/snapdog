export const locales = ["en", "de", "fr", "es", "it", "nl", "pl", "pt"] as const;
export type Locale = (typeof locales)[number];
export const defaultLocale: Locale = "en";
