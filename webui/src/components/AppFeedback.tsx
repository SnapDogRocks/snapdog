"use client";
import { useEffect, useState } from "react";
import { useTranslations } from "next-intl";
import { Button } from "@/components/ui/button";
export function LoadFailure({ onRetry }: { onRetry: () => void }) {
    const t = useTranslations();
    return (
        <main
            className="flex flex-1 flex-col items-center justify-center gap-4 p-8 text-center"
            role="alert"
        >
            <h1 className="text-xl font-semibold">{t("current.unavailable")}</h1>
            <p className="max-w-md text-muted-foreground">{t("current.loadFailed")}</p>
            <Button onClick={onRetry}>{t("zone.retry")}</Button>
        </main>
    );
}
export function ApiFeedback() {
    const t = useTranslations();
    const [error, setError] = useState<string | null>(null);
    useEffect(() => {
        const receive = (event: Event) => {
            const detail: unknown = (event as CustomEvent<unknown>).detail;
            setError(
                typeof detail === "object" &&
                    detail !== null &&
                    "message" in detail &&
                    typeof detail.message === "string"
                    ? detail.message
                    : "",
            );
        };
        window.addEventListener("snapdog:api-error", receive);
        return () => {
            window.removeEventListener("snapdog:api-error", receive);
        };
    }, []);
    if (error === null) return null;
    return (
        <div
            className="fixed bottom-4 left-4 right-4 z-[90] mx-auto flex max-w-lg items-start gap-3 rounded-xl border border-destructive/40 bg-popover p-4 text-popover-foreground shadow-xl"
            role="alert"
        >
            <div className="min-w-0 flex-1">
                <p className="font-medium">{t("current.commandFailed")}</p>
                {error && (
                    <details className="mt-1 text-xs">
                        <summary>{t("current.details")}</summary>
                        <p className="mt-2 break-all">{error}</p>
                    </details>
                )}
            </div>
            <Button
                variant="ghost"
                size="sm"
                onClick={() => {
                    setError(null);
                }}
            >
                {t("current.dismiss")}
            </Button>
        </div>
    );
}
