"use client";
import { useState } from "react";
import { useTranslations } from "next-intl";
import { useAppStore } from "@/stores/useAppStore";
import { useAppSession } from "@/hooks/useAppSession";
import { useSpeakerMoves } from "@/hooks/useSpeakerMoves";
import { useSpeakerDrag } from "@/hooks/useSpeakerDrag";
import { ApiKeyPrompt } from "@/components/ApiKeyPrompt";
import { LoadFailure, ApiFeedback } from "@/components/AppFeedback";
import { EmptyState } from "@/components/EmptyState";
import { ConnectionStatus } from "@/components/ConnectionStatus";
import { ThemeToggle } from "@/components/ThemeToggle";
import { LocalePicker } from "@/components/LocalePicker";
import { AboutButton } from "@/components/AboutButton";
import { ConnectButton } from "@/components/ConnectButton";
import { PlaylistBrowser } from "@/components/PlaylistBrowser";
import { Player } from "./Player";
import { SpeakerPanel } from "./SpeakerPanel";
export function CurrentApp() {
    const t = useTranslations();
    const {
        zones,
        clients,
        selectedZone,
        selectZone,
        needsAuth,
        isLoading,
        loadError,
        loadAll,
        isConnected,
    } = useAppStore();
    const { serverName, retryIn } = useAppSession();
    const [selection, setSelection] = useState<number[]>([]);
    const transfers = useSpeakerMoves();
    const move = (ids: number[], target: number) => {
        void transfers.move(ids, target);
    };
    const drag = useSpeakerDrag(move);
    const zoneList = Array.from(zones.values());
    const zone = zones.get(selectedZone) ?? zoneList[0];
    const allClients = Array.from(clients.values());
    const zoneClients = allClients.filter((c) => c.zone_index === zone?.index);
    const select = (id: number) => {
        selectZone(id);
        setSelection([]);
    };
    if (needsAuth)
        return (
            <ApiKeyPrompt
                onAuthenticated={() => {
                    void loadAll();
                }}
            />
        );
    if (isLoading)
        return (
            <main className="current-loading" aria-busy="true">
                <img src="/assets/snapdog-icon.svg" alt="SnapDog" />
                <p role="status">{t("current.loading")}</p>
            </main>
        );
    if (loadError && !zone)
        return (
            <LoadFailure
                onRetry={() => {
                    void loadAll();
                }}
            />
        );
    if (!zone) return <EmptyState />;
    const notice = transfers.notice;
    return (
        <div className="current-app">
            <aside className="current-sidebar">
                <a className="current-skip" href="#main-content">
                    {t("app.skipToContent")}
                </a>
                <div className="current-brand">
                    <img src="/assets/snapdog-icon.svg" alt="" />
                    <div>
                        <strong>snapdog</strong>
                        {serverName !== "SnapDog" && <p>{serverName}</p>}
                    </div>
                </div>
                <div className="current-zone-heading">
                    <span className="current-eyebrow">{t("zone.zones")}</span>
                    <span>{zoneList.length.toString().padStart(2, "0")}</span>
                </div>
                <nav className="current-zone-list" aria-label={t("zone.navigation")}>
                    {zoneList.map((item) => {
                        const count = allClients.filter((c) => c.zone_index === item.index).length;
                        const target = drag.drag?.target === item.index;
                        return (
                            <button
                                key={item.index}
                                className={`current-zone ${zone.index === item.index ? "is-active" : ""} ${target ? "is-drop-target" : ""}`}
                                data-drop-zone={item.index}
                                onClick={() => {
                                    select(item.index);
                                }}
                                aria-current={zone.index === item.index ? "true" : undefined}
                            >
                                <span className="current-zone-icon" aria-hidden="true">
                                    {item.icon || "◈"}
                                </span>
                                <span className="current-zone-copy">
                                    <strong>{item.name}</strong>
                                    <small>
                                        {target
                                            ? t("current.dropHere")
                                            : t("current.speakerCount", { count })}
                                    </small>
                                </span>
                                {item.playback === "playing" && (
                                    <span
                                        className="current-equalizer"
                                        aria-label={t("zone.playing")}
                                    >
                                        <i />
                                        <i />
                                        <i />
                                    </span>
                                )}
                            </button>
                        );
                    })}
                </nav>
                <div className="current-sidebar-bottom">
                    <span className="current-status">
                        <span className={`current-dot ${isConnected ? "is-playing" : ""}`} />
                        {t(isConnected ? "app.connected" : "app.disconnected")}
                    </span>
                </div>
            </aside>
            <div className="current-workspace">
                <main id="main-content" tabIndex={-1} className="current-main">
                    <div className="current-page-heading">
                        <div>
                            <h1>{zone.name}</h1>
                            <p className="current-zone-meta">
                                {t("current.speakerCount", { count: zoneClients.length })}
                                {zone.presence && (
                                    <span>
                                        {" "}
                                        ·{" "}
                                        {t(
                                            zone.presenceTimerActive
                                                ? "zone.presenceTimerActive"
                                                : "zone.presenceDetected",
                                        )}
                                    </span>
                                )}
                            </p>
                        </div>
                        <div className="current-tools">
                            <ConnectButton />
                            <LocalePicker />
                            <ThemeToggle />
                            <AboutButton />
                        </div>
                    </div>
                    {loadError && (
                        <div role="alert" className="mb-4 rounded-lg border border-destructive p-4">
                            <p>{t("current.loadFailed")}</p>
                            <button
                                className="current-text-button"
                                onClick={() => {
                                    void loadAll();
                                }}
                            >
                                {t("zone.retry")}
                            </button>
                        </div>
                    )}
                    <Player key={zone.index} zone={zone} />
                    <section className="current-library" aria-label={t("playlist.browse")}>
                        <PlaylistBrowser key={zone.index} zone={zone} />
                    </section>
                    <div id="speaker-panel">
                        <SpeakerPanel
                            key={zone.index}
                            zone={zone}
                            zones={zoneList}
                            clients={zoneClients}
                            selection={selection}
                            select={setSelection}
                            drag={drag}
                            busy={transfers.busy}
                            canUndo={transfers.canUndo}
                            onUndo={() => {
                                void transfers.undo();
                            }}
                            onMove={move}
                        />
                    </div>
                </main>
            </div>
            {(notice !== null || transfers.busy) && (
                <div className="current-transfer-toast" role="status" aria-live="polite">
                    <div>
                        {transfers.busy
                            ? t(transfers.undoing ? "current.restoring" : "current.moving")
                            : notice && (
                                  <>
                                      <p>
                                          {t(notice.undo ? "current.restored" : "current.moved", {
                                              count: notice.result.moved.length,
                                          })}
                                      </p>
                                      {notice.result.failed.length > 0 && (
                                          <p>
                                              {t(
                                                  notice.undo
                                                      ? "current.undoFailed"
                                                      : "current.moveFailed",
                                                  {
                                                      count: notice.result.failed.length,
                                                  },
                                              )}
                                          </p>
                                      )}
                                      {notice.undo && notice.result.skipped.length > 0 && (
                                          <p>
                                              {t("current.undoSkipped", {
                                                  count: notice.result.skipped.length,
                                              })}
                                          </p>
                                      )}
                                  </>
                              )}
                    </div>
                </div>
            )}
            {drag.drag && (
                <div
                    className="current-drag-ghost"
                    style={{
                        left: Math.min(drag.drag.x + 16, window.innerWidth - 170),
                        top: Math.max(8, drag.drag.y - 50),
                    }}
                    aria-hidden="true"
                >
                    {t("current.speakerCount", { count: drag.drag.ids.length })}
                    <span>
                        {drag.drag.target !== null
                            ? t("current.dropHere")
                            : t("current.dragToZone")}
                    </span>
                </div>
            )}
            <ConnectionStatus retryIn={retryIn} />
            <ApiFeedback />
        </div>
    );
}
