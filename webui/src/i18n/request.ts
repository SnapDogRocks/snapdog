import { getRequestConfig } from "next-intl/server";
import { defaultLocale } from "./config";

export default getRequestConfig(async () => {
  const locale = defaultLocale;
  // TS can't resolve a template-literal dynamic import to a typed module;
  // the shape is guaranteed by the JSON files under messages/.
  const mod = (await import(`../../messages/${locale}.json`)) as { default: Record<string, unknown> };
  return {
    locale,
    messages: mod.default,
  };
});
