"use client";

import { useState } from "react";
import { useTranslations } from "next-intl";
import { api } from "@/lib/api";
import { logApiError } from "@/lib/log-api-error";
import { useEqEnabled } from "@/hooks/useEqEnabled";
import { MIME } from "@/hooks/useClientDrop";
import { useAppStore, type ZoneState } from "@/stores/useAppStore";
import type { ClientInfo } from "@/lib/types";
import { VolumeSlider } from "@/components/VolumeSlider";
import { EqOverlay } from "@/components/EqOverlay";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";

function ClientCard({ client }: { client: ClientInfo }) {
  const t = useTranslations("client");
  const zones = useAppStore((s) => s.zones);
  const otherZones = Array.from(zones.values()).filter((z) => z.index !== client.zone_index);
  const [showEq, setShowEq] = useState(false);
  const [eqEnabled, setEqEnabled] = useEqEnabled(client.is_snapdog ? { clientId: client.index } : {});

  return (
    <div
      className="relative flex items-stretch gap-2 px-3 py-2.5 rounded-lg bg-muted shadow-[inset_0_2px_4px_rgba(0,0,0,0.15)] border border-border/50 cursor-grab hover:border-primary/30 transition-colors"
      draggable
      aria-roledescription="draggable client"
      onDragStart={(e) => {
        if ((e.target as HTMLElement).closest("[data-slot=slider]") || (e.target as HTMLElement).closest("[data-menu]")) {
          e.preventDefault();
          return;
        }
        e.dataTransfer.setData(MIME, String(client.index));
        e.dataTransfer.effectAllowed = "move";
      }}
    >
      {/* Drag handle — visual indicator */}
      <div className="shrink-0 flex items-center text-muted-foreground/30" aria-hidden="true">        <div className="flex flex-col gap-[3px]">
          <div className="flex gap-[3px]"><div className="size-[3px] rounded-full bg-current" /><div className="size-[3px] rounded-full bg-current" /></div>
          <div className="flex gap-[3px]"><div className="size-[3px] rounded-full bg-current" /><div className="size-[3px] rounded-full bg-current" /></div>
          <div className="flex gap-[3px]"><div className="size-[3px] rounded-full bg-current" /><div className="size-[3px] rounded-full bg-current" /></div>
          <div className="flex gap-[3px]"><div className="size-[3px] rounded-full bg-current" /><div className="size-[3px] rounded-full bg-current" /></div>
        </div>
      </div>
      <div className="min-w-0 flex-1 space-y-1.5">
        {/* Name row: icon + connection indicator + name + menu */}
        <div className="flex items-center gap-1.5">
          <div className={`size-2 shrink-0 ${client.connected ? "rounded-full bg-green-500" : "rotate-45 bg-destructive"}`} />
          <span className="sr-only">{client.connected ? t("connected") : t("disconnected")}</span>
          {client.icon && <span className="text-lg shrink-0">{client.icon}</span>}
          <span className="text-sm font-medium truncate">{client.name}</span>
          {otherZones.length > 0 && (
            <div className="ml-auto" data-menu>
              <DropdownMenu>
                <DropdownMenuTrigger asChild>
                  <button
                    className="p-1 -m-1 text-muted-foreground hover:text-foreground transition-colors"
                    aria-label={t("moveTo")}
                  >
                    <span className="text-sm tracking-wider">⋯</span>
                  </button>
                </DropdownMenuTrigger>
                <DropdownMenuContent align="end" className="w-48">
                  <DropdownMenuLabel>{t("moveToLabel")}</DropdownMenuLabel>
                  {otherZones.map((z) => (
                    <DropdownMenuItem
                      key={z.index}
                      onClick={() => {
                        api.clients.setZone(client.index, z.index).catch(logApiError);
                      }}
                    >
                      <span className="mr-2">{z.icon}</span>
                      <span>{z.name}</span>
                    </DropdownMenuItem>
                  ))}
                </DropdownMenuContent>
              </DropdownMenu>
            </div>
          )}
        </div>
        {/* Volume + EQ */}
        <div className="flex items-center gap-1">
          <div className="flex-1">
            <VolumeSlider
              volume={client.volume}
              muted={client.muted}
              onVolumeChange={(v) => api.clients.setVolume(client.index, v).catch(logApiError)}
              onMuteToggle={() => api.clients.toggleMute(client.index).catch(logApiError)}
              onUnmute={() => api.clients.setMute(client.index, false).catch(logApiError)}
              max={client.max_volume}
              compact
            />
          </div>
          {client.is_snapdog && (
            <button
              onClick={() => setShowEq(true)}
              className={`text-[10px] transition-colors px-1 ${eqEnabled ? "text-orange-500 font-bold" : "text-muted-foreground hover:text-foreground"}`}
              aria-label={`EQ ${client.name}`}
            >
              EQ
            </button>
          )}
        </div>
      </div>
      {showEq && <EqOverlay clientId={client.index} label={client.name} onClose={(enabled) => { setShowEq(false); setEqEnabled(enabled); }} />}
    </div>
  );
}

const SHOW_OFFLINE_KEY = "snapdog-show-offline";

export function ClientList({ zone }: { zone: ZoneState }) {
  const t = useTranslations("client");
  const [showOffline, setShowOffline] = useState(() => typeof window !== "undefined" && localStorage.getItem(SHOW_OFFLINE_KEY) === "true");
  const clients = useAppStore((s) => s.clients);

  const zoneClients = Array.from(clients.values()).filter((c) => c.zone_index === zone.index);
  const connectedClients = zoneClients.filter((c) => c.connected);
  const offlineCount = zoneClients.length - connectedClients.length;
  const visibleClients = showOffline ? zoneClients : connectedClients;

  const toggleOffline = () => {
    const next = !showOffline;
    setShowOffline(next);
    localStorage.setItem(SHOW_OFFLINE_KEY, String(next));
  };

  if (visibleClients.length === 0 && offlineCount === 0) return null;

  return (
    <div className="w-full">
      <div className="flex items-center px-3 py-2">
        <span className="text-xs text-muted-foreground">
          {t("clients", { count: visibleClients.length })}
        </span>
        {offlineCount > 0 && (
          <button
            onClick={toggleOffline}
            className="ml-auto text-[10px] text-muted-foreground/60 hover:text-muted-foreground transition-colors"
            aria-label={showOffline ? t("hideOffline") : t("showOffline", { count: offlineCount })}
            aria-pressed={showOffline}
          >
            {showOffline ? t("hideOffline") : t("showOffline", { count: offlineCount })}
          </button>
        )}
      </div>
      {visibleClients.length > 0 && (
        <div className="grid grid-cols-[repeat(auto-fill,minmax(14rem,1fr))] gap-1">
          {visibleClients.map((c) => (
            <div key={c.index}>
              <ClientCard client={c} />
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
