import { useTranslations } from "next-intl";
import { useAppStore } from "@/stores/useAppStore";

export function ConnectionStatus({ retryIn }: { retryIn: number }) {
  const t = useTranslations("connection");
  const isConnected = useAppStore((s) => s.isConnected);
  const serverGoingAway = useAppStore((s) => s.serverGoingAway);
  const countdown = Math.max(0, retryIn);

  if (isConnected) return null;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-background/60 backdrop-blur-sm" role="alert" aria-live="assertive">
      <div className="bg-card border border-border rounded-xl p-6 shadow-lg text-center space-y-3">
        <div className="size-8 border-2 border-primary border-t-transparent rounded-full animate-spin mx-auto" />
        <p className="text-sm font-medium">{serverGoingAway ? t("serverRestarting") : t("lost")}</p>
        <p className="text-xs text-muted-foreground">
          {countdown > 0 ? t("retryingIn", { seconds: countdown }) : t("reconnecting")}
        </p>
      </div>
    </div>
  );
}
