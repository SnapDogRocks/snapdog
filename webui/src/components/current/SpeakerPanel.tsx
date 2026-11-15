"use client";
import { useRef, useState } from "react";
import { useTranslations } from "next-intl";
import type { ClientInfo } from "@/lib/types";
import type { ZoneState } from "@/stores/useAppStore";
import type { useSpeakerDrag } from "@/hooks/useSpeakerDrag";
import { VolumeSlider } from "@/components/VolumeSlider";
import { EqOverlay } from "@/components/EqOverlay";
import { api } from "@/lib/api";
import { logApiError } from "@/lib/log-api-error";
import {
    DropdownMenu,
    DropdownMenuTrigger,
    DropdownMenuContent,
    DropdownMenuItem,
} from "@/components/ui/dropdown-menu";
interface Props {
    zone: ZoneState;
    zones: ZoneState[];
    clients: ClientInfo[];
    selection: number[];
    select: (ids: number[]) => void;
    drag: ReturnType<typeof useSpeakerDrag>;
    busy: boolean;
    canUndo: boolean;
    onUndo: () => void;
    onMove: (ids: number[], zone: number) => void;
}
export function SpeakerPanel({
    zone,
    zones,
    clients,
    selection,
    select,
    drag,
    busy,
    canUndo,
    onUndo,
    onMove,
}: Props) {
    const t = useTranslations();
    const [menuOpen, setMenuOpen] = useState(false);
    const [portalContainer, setPortalContainer] = useState<HTMLElement | null>(null);
    const [eq, setEq] = useState<ClientInfo | null>(null);
    const heading = useRef<HTMLHeadingElement>(null);
    const opener = useRef<HTMLButtonElement | null>(null);
    const movedFromMenu = useRef(false);
    const tapPending = useRef(false);
    const targets = zones.filter((z) => z.index !== zone.index);
    const selected = clients.filter((c) => selection.includes(c.index)).map((c) => c.index);
    const toggle = (id: number) => {
        setMenuOpen(false);
        select(selected.includes(id) ? selected.filter((i) => i !== id) : [...selected, id]);
    };
    return (
        <section
            ref={setPortalContainer}
            className="current-speakers"
            aria-labelledby="speaker-title"
        >
            <div className="current-section-heading">
                <div>
                    <h2 id="speaker-title" ref={heading} tabIndex={-1}>
                        {t("current.speakers")} <span>{clients.length}</span>
                    </h2>
                </div>
                <div className="current-speaker-actions">
                    {canUndo && (
                        <button
                            className="current-undo"
                            disabled={busy}
                            aria-label={t("current.undo")}
                            title={t("current.undo")}
                            onClick={() => {
                                heading.current?.focus({ preventScroll: true });
                                onUndo();
                            }}
                        >
                            <svg
                                width="18"
                                height="18"
                                viewBox="0 0 24 24"
                                fill="none"
                                stroke="currentColor"
                                strokeWidth="1.7"
                                strokeLinecap="round"
                                strokeLinejoin="round"
                                aria-hidden="true"
                            >
                                <path d="m9 14-5-5 5-5" />
                                <path d="M4 9h9a7 7 0 0 1 0 14h-3" />
                            </svg>
                        </button>
                    )}
                    {selected.length > 0 ? (
                        <>
                            <button
                                className="current-clear-selection"
                                disabled={busy}
                                aria-label={t("current.clearSelection")}
                                title={t("current.clearSelection")}
                                onClick={() => {
                                    setMenuOpen(false);
                                    select([]);
                                    heading.current?.focus({ preventScroll: true });
                                }}
                            >
                                <span aria-hidden="true">×</span>
                            </button>
                            <DropdownMenu modal={false} open={menuOpen} onOpenChange={setMenuOpen}>
                                <DropdownMenuTrigger asChild>
                                    <button
                                        className="current-move-trigger"
                                        disabled={busy}
                                        aria-label={t("client.moveTo")}
                                        onPointerDown={() => {
                                            opener.current = null;
                                        }}
                                        onKeyDown={() => {
                                            opener.current = null;
                                        }}
                                    >
                                        {t("current.move")} <span aria-hidden="true">⌄</span>
                                    </button>
                                </DropdownMenuTrigger>
                                <DropdownMenuContent
                                    align="end"
                                    portalContainer={portalContainer}
                                    className="current-zone-menu"
                                    aria-label={t("client.moveTo")}
                                    onCloseAutoFocus={(event) => {
                                        if (movedFromMenu.current) {
                                            event.preventDefault();
                                            heading.current?.focus({ preventScroll: true });
                                        } else if (opener.current?.isConnected) {
                                            event.preventDefault();
                                            opener.current.focus({ preventScroll: true });
                                        }
                                        movedFromMenu.current = false;
                                        opener.current = null;
                                    }}
                                >
                                    {targets.map((target) => (
                                        <DropdownMenuItem
                                            key={target.index}
                                            disabled={busy}
                                            onSelect={() => {
                                                movedFromMenu.current = true;
                                                onMove(selected, target.index);
                                            }}
                                        >
                                            <span aria-hidden="true">{target.icon || "◈"}</span>
                                            <span>{target.name}</span>
                                        </DropdownMenuItem>
                                    ))}
                                    {!targets.length && (
                                        <DropdownMenuItem disabled>
                                            {t("current.noOtherZone")}
                                        </DropdownMenuItem>
                                    )}
                                </DropdownMenuContent>
                            </DropdownMenu>
                        </>
                    ) : (
                        <button
                            className="current-text-button"
                            disabled={busy || !clients.length}
                            onClick={() => {
                                select(clients.map((c) => c.index));
                            }}
                        >
                            {t("current.selectAll")}
                        </button>
                    )}
                </div>
            </div>
            <p className="sr-only" id="speaker-drag-help">
                {t("current.dragHint")}
            </p>
            <div className="current-speaker-grid">
                {clients.map((client) => (
                    <article
                        className={`current-speaker ${selected.includes(client.index) ? "is-selected" : ""}`}
                        key={client.index}
                        data-speaker={client.index}
                    >
                        <div className="current-speaker-top">
                            <input
                                type="checkbox"
                                checked={selected.includes(client.index)}
                                onChange={() => {
                                    toggle(client.index);
                                }}
                                disabled={busy}
                                aria-label={t("current.selectSpeaker", { name: client.name })}
                            />
                            <div className="current-speaker-name">
                                <h3>{client.name}</h3>
                                <p>
                                    <span
                                        className={`current-dot ${client.connected ? "is-playing" : ""}`}
                                    />
                                    {t(
                                        client.connected
                                            ? "client.connected"
                                            : "client.disconnected",
                                    )}
                                </p>
                            </div>
                            <button
                                className="current-drag-handle"
                                disabled={busy || !targets.length}
                                aria-label={t("current.moveSpeaker", { name: client.name })}
                                aria-describedby="speaker-drag-help"
                                onPointerDown={(e) => {
                                    tapPending.current = false;
                                    const ids = selected.includes(client.index)
                                        ? selected
                                        : [client.index];
                                    select(ids);
                                    drag.start(e, ids);
                                }}
                                onPointerMove={drag.move}
                                onPointerUp={(event) => {
                                    tapPending.current = drag.end(event);
                                }}
                                onPointerCancel={(event) => {
                                    tapPending.current = false;
                                    drag.cancel(event);
                                }}
                                onLostPointerCapture={drag.cancel}
                                onClick={(e) => {
                                    if (e.detail === 0 || tapPending.current) {
                                        if (!selected.includes(client.index))
                                            select([client.index]);
                                        opener.current = e.currentTarget;
                                        setMenuOpen(true);
                                    }
                                    tapPending.current = false;
                                }}
                            >
                                <span aria-hidden="true">⠿</span>
                            </button>
                        </div>
                        <div className="current-speaker-volume">
                            <VolumeSlider
                                label={t("current.volumeFor", { name: client.name })}
                                volume={client.volume}
                                muted={client.muted}
                                max={client.max_volume}
                                compact
                                onVolumeChange={(v) => {
                                    api.clients.setVolume(client.index, v).catch(logApiError);
                                }}
                                onMuteToggle={() => {
                                    api.clients.toggleMute(client.index).catch(logApiError);
                                }}
                                onUnmute={() => {
                                    api.clients.setMute(client.index, false).catch(logApiError);
                                }}
                            />
                            {client.is_snapdog && (
                                <button
                                    className="current-text-button"
                                    onClick={() => {
                                        setEq(client);
                                    }}
                                    aria-label={t("eq.title", { zone: client.name })}
                                >
                                    EQ
                                </button>
                            )}
                        </div>
                    </article>
                ))}
            </div>
            {!clients.length && (
                <div className="current-empty-speakers">{t("current.noSpeakers")}</div>
            )}
            {eq && (
                <EqOverlay
                    clientId={eq.index}
                    label={eq.name}
                    onClose={() => {
                        setEq(null);
                    }}
                />
            )}
        </section>
    );
}
