import { useState } from "react";
import { useTranslations } from "next-intl";
import { useFocusTrap } from "@/hooks/useFocusTrap";
import { setApiKey } from "@/lib/auth";
import { Button } from "@/components/ui/button";

interface ApiKeyPromptProps {
  onAuthenticated: () => void;
}

export function ApiKeyPrompt({ onAuthenticated }: ApiKeyPromptProps) {
  const t = useTranslations("auth");
  const trapRef = useFocusTrap<HTMLDivElement>();
  const [key, setKey] = useState("");
  const [error, setError] = useState(false);
  const [checking, setChecking] = useState(false);

  const submit = async () => {
    if (!key.trim()) return;
    setChecking(true);
    setError(false);
    try {
      const res = await fetch("/api/v1/system/status", {
        headers: { Authorization: `Bearer ${key.trim()}` },
      });
      if (res.ok) {
        setApiKey(key.trim());
        onAuthenticated();
      } else {
        setError(true);
      }
    } catch {
      setError(true);
    } finally {
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
            {t.rich("description", {
              configKey: () => <code className="text-xs bg-muted px-1 py-0.5 rounded">api_keys</code>,
              configFile: () => <code className="text-xs bg-muted px-1 py-0.5 rounded">snapdog.toml</code>,
            })}
          </p>
        </div>
        <div className="space-y-2">
          <input
            type="password"
            placeholder={t("placeholder")}
            value={key}
            onChange={(e: React.ChangeEvent<HTMLInputElement>) => { setKey(e.target.value); setError(false); }}
            onKeyDown={(e: React.KeyboardEvent) => { if (e.key === "Enter") void submit(); }}
            autoFocus
            aria-label={t("placeholder")}
            className="flex h-10 w-full rounded-md border border-input bg-background px-3 py-2 text-sm ring-offset-background placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          />
          {error && (
            <p className="text-sm text-destructive" role="alert">{t("invalid")}</p>
          )}
        </div>
        <Button onClick={() => { void submit(); }} disabled={checking || !key.trim()} className="w-full">
          {checking ? t("checking") : t("submit")}
        </Button>
      </div>
    </div>
  );
}
