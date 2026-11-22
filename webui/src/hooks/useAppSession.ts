"use client";
import { useCallback, useEffect, useState } from "react";
import { useAppStore } from "@/stores/useAppStore";
import { useWebSocket } from "@/hooks/useWebSocket";
import { useKeyboardShortcuts } from "@/hooks/useKeyboardShortcuts";
import { api } from "@/lib/api";
import { setApiKey } from "@/lib/auth";
import { trackFromZoneChanged } from "@/lib/ws-zone";
import { publishEqChange } from "@/lib/eq-events";
import type { WsNotification } from "@/lib/types";
/** Apply one WebSocket event to the store. */
export function applyNotification(n: WsNotification): void {
    const store = useAppStore.getState();
    switch (n.type) {
        case "zone_changed":
            store.updateZone(n.zone, {
                playback: n.playback,
                volume: n.volume,
                muted: n.muted,
                source: n.source,
                shuffle: n.shuffle,
                repeat: n.repeat,
                track: trackFromZoneChanged(n),
            });
            break;
        case "playlists_changed":
            store.bumpPlaylists();
            break;
        case "zone_volume_changed":
            store.updateZoneVolume(n.zone, n.volume, n.muted);
            break;
        case "zone_progress":
            store.updateZoneProgress(
                n.zone,
                n.position_ms,
                n.duration_ms,
                n.buffered_ms ?? null,
            );
            break;
        case "client_state_changed":
            store.updateClient(n.client, {
                // Absent in events of older servers: keep the name and latency then.
                ...(n.name ? { name: n.name } : {}),
                ...(n.latency_ms === undefined ? {} : { latency_ms: n.latency_ms }),
                volume: n.volume,
                muted: n.muted,
                connected: n.connected,
                zone_index: n.zone,
                is_snapdog: n.is_snapdog,
            });
            break;
        case "zone_presence_changed":
            store.updateZonePresence(n.zone, n.presence, n.enabled, n.timer_active);
            break;
        case "zone_eq_changed":
            store.updateZoneEq(n.zone, n.enabled, n.bands, n.preset);
            publishEqChange(`zone:${n.zone}`, { enabled: n.enabled, bands: n.bands, preset: n.preset ?? null });
            break;
        case "playback_error":
            store.setZoneError(n.zone, {
                message: n.message,
                details: n.details,
                recoverable: n.recoverable,
            });
            break;
        case "client_eq_changed":
            store.updateClient(n.client, { eq_enabled: n.enabled });
            publishEqChange(`client:${n.client}`, { enabled: n.enabled, bands: n.bands, preset: n.preset ?? null });
            break;
        case "client_speaker_changed":
            // The store keeps no speaker correction state; an open EQ panel follows it.
            publishEqChange(`speaker:${n.client}`, { enabled: n.enabled, bands: n.bands, preset: n.preset ?? null });
            break;
        default: {
            // A server event without a case here fails the type check. At run time an
            // event of a newer server is ignored.
            const unhandled: never = n;
            return unhandled;
        }
    }
}

export function useAppSession() {
    const needsAuth = useAppStore((s) => s.needsAuth);
    const [serverName, setServerName] = useState("SnapDog");
    const notification = useCallback((n: WsNotification) => {
        applyNotification(n);
    }, []);
    const { isConnected, serverGoingAway, retryIn } = useWebSocket(
        notification,
        () => {
            void useAppStore.getState().loadAll({ fresh: true });
            useAppStore.getState().bumpPlaylists();
        },
        !needsAuth,
    );
    useKeyboardShortcuts();
    useEffect(() => {
        const unauthorized = () => {
            useAppStore.setState({ needsAuth: true });
        };
        window.addEventListener("snapdog:unauthorized", unauthorized);
        const params = new URLSearchParams(window.location.search);
        const auth = params.get("auth");
        if (auth) {
            setApiKey(auth);
            params.delete("auth");
            const search = params.toString();
            window.history.replaceState(
                {},
                "",
                `${window.location.pathname}${search ? `?${search}` : ""}${window.location.hash}`,
            );
        }
        void useAppStore.getState().loadAll();
        return () => {
            window.removeEventListener("snapdog:unauthorized", unauthorized);
        };
    }, []);
    useEffect(() => {
        useAppStore.getState().setConnected(isConnected, serverGoingAway);
    }, [isConnected, serverGoingAway]);
    useEffect(() => {
        if (needsAuth) return;
        let stale = false;
        api.system
            .version()
            .then((v) => {
                if (stale) return;
                setServerName(v.name);
                document.title = v.name === "SnapDog" ? "SnapDog" : `SnapDog — ${v.name}`;
            })
            .catch(() => {
                /* System information is optional; retain the default title. */
            });
        return () => {
            stale = true;
        };
    }, [needsAuth]);
    return { serverName, retryIn };
}
