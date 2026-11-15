import { useRef, useState } from "react";
import { useTranslations } from "next-intl";
import { useFocusTrap } from "@/hooks/useFocusTrap";
import { setApiKey } from "@/lib/auth";
import { Button } from "@/components/ui/button";

interface ApiKeyPromptProps {
  onAuthenticated: () => void;
}

export function ApiKeyPrompt({ onAuthenticated }: ApiKeyPromptProps) {
  const t = useTranslations("auth");
  const common = useTranslations("current");
  const pending = useRef(false);
  const trapRef = useFocusTrap<HTMLDivElement>();
  const [key, setKey] = useState("");
  const [error, setError] = useState<"invalid" | "unavailable" | null>(null);
  const [checking, setChecking] = useState(false);

  const submit = async () => {
    if (!key.trim() || pending.current) return;
    pending.current = true;
    setChecking(true);
    setError(null);
    try {
      const res = await fetch("/api/v1/system/status", {
        headers: { Authorization: `Bearer ${key.trim()}` },
      });
      if (res.ok) {
        setApiKey(key.trim());
        onAuthenticated();
      } else {
        setError(res.status === 401 || res.status === 403 ? "invalid" : "unavailable");
      }
    } catch {
      setError("unavailable");
    } finally {
      pending.current = false;
      setChecking(false);
    }
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center" role="dialog" aria-modal="true" aria-label={t("title")}>
      <div className="absolute inset-0 bg-background/80 backdrop-blur-sm" role="presentation" />
      <div ref={trapRef} className="relative z-10 w-full max-w-sm mx-4 rounded-2xl border border-border bg-card p-6 shadow-xl space-y-4">
        <div className="space-y-1">
          <h2 className="text-lg font-semibold">{t("title")}</h2>
          <p className="text-sm text-muted-foreground">
            {t("description", { configKey: "api_keys", configFile: "snapdog.toml" })}
          </p>
        </div>
        <div className="space-y-2">
          <input
            type="password"
            placeholder={t("placeholder")}
            value={key}
            onChange={(e: React.ChangeEvent<HTMLInputElement>) => { setKey(e.target.value); setError(null); }}
            onKeyDown={(e: React.KeyboardEvent) => { if (e.key === "Enter") void submit(); }}
            autoFocus
            aria-label={t("placeholder")}
            className="flex h-10 w-full rounded-md border border-input bg-background px-3 py-2 text-sm ring-offset-background placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          />
          {error && (
            <p className="text-sm text-destructive" role="alert">{error === "invalid" ? t("invalid") : common("authUnavailable")}</p>
          )}
        </div>
        <Button onClick={() => { void submit(); }} disabled={checking || !key.trim()} className="w-full">
          {checking ? t("checking") : t("submit")}
        </Button>
      </div>
    </div>
  );
}
