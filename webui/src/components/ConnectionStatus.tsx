import { useTranslations } from "next-intl";
import { useAppStore } from "@/stores/useAppStore";
export function ConnectionStatus({ retryIn }: { retryIn: number }) {
    const t = useTranslations();
    const isConnected = useAppStore((s) => s.isConnected);
    const serverGoingAway = useAppStore((s) => s.serverGoingAway);
    if (isConnected) return null;
    return (
        <div
            className="connection-notice fixed bottom-4 left-1/2 z-40 max-w-[calc(100%-2rem)] -translate-x-1/2 rounded-xl border border-border bg-popover px-4 py-3 text-sm text-popover-foreground shadow-lg"
            role="status"
            aria-live="polite"
        >
            <strong>
                {serverGoingAway ? t("connection.serverRestarting") : t("connection.lost")}
            </strong>
            <span className="ml-2">
                {retryIn > 0
                    ? t("connection.retryingIn", { seconds: retryIn })
                    : t("connection.reconnecting")}
            </span>
            <p className="mt-1 text-xs text-muted-foreground">{t("current.stale")}</p>
        </div>
    );
}
