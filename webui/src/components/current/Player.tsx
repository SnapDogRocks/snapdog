"use client";
import { useState } from "react";
import { useTranslations } from "next-intl";
import { useAppStore, type ZoneState } from "@/stores/useAppStore";
import { api } from "@/lib/api";
import { logApiError } from "@/lib/log-api-error";
import { NowPlaying } from "@/components/NowPlaying";
import { TransportControls } from "@/components/TransportControls";
import { SeekBar } from "@/components/SeekBar";
import { VolumeSlider } from "@/components/VolumeSlider";
import { ShuffleRepeat } from "@/components/ShuffleRepeat";
import { EqOverlay } from "@/components/EqOverlay";
import { PlaybackErrorBanner } from "@/components/PlaybackErrorBanner";
export function Player({ zone }: { zone: ZoneState }) {
    const t = useTranslations();
    const [eq, setEq] = useState(false);
    const active = zone.source !== "idle";
    const library = zone.source === "subsonic_playlist" || zone.source === "subsonic_track";
    return (
        <section className="current-player" aria-label={t("current.nowPlaying")}>
            <div className="current-art">
                <NowPlaying zone={zone} />
            </div>
            <div className="current-player-content">
                <div className="current-track">
                    <div className="current-track-heading">
                        <h2>
                            {active
                                ? (zone.track?.title ?? "") || t("zone.unknownTitle")
                                : t("zone.noAudio")}
                        </h2>
                        <div className="current-eyebrow current-source">
                            <span
                                className={`current-dot ${zone.playback === "playing" ? "is-playing" : ""}`}
                            />
                            {active ? t(`source.${zone.source}`) : t("zone.idle")}
                        </div>
                    </div>
                    {active && <p>{(zone.track?.artist ?? "") || t("zone.unknownArtist")}</p>}
                    {active && zone.track?.album && (
                        <p className="current-album">{zone.track.album}</p>
                    )}
                </div>
                <PlaybackErrorBanner zone={zone} />
                <div className="current-controls-row">
                    <div className="current-transport">
                        <TransportControls zone={zone} />
                        {library && <ShuffleRepeat zone={zone} />}
                    </div>
                    <PlayerVolume
                        zone={zone}
                        onEq={() => {
                            setEq(true);
                        }}
                    />
                </div>
                <div className="current-progress">
                    {active &&
                        (zone.track?.duration_ms ? (
                            <SeekBar zone={zone} />
                        ) : (
                            <div className="current-live">
                                <span />
                                {t("current.live")}
                            </div>
                        ))}
                </div>
            </div>
            {eq && (
                <EqOverlay
                    initialEnabled={zone.eqEnabled ?? false}
                    zoneId={zone.index}
                    label={zone.name}
                    onClose={(enabled) => {
                        setEq(false);
                        useAppStore.getState().updateZoneEq(zone.index, enabled);
                    }}
                />
            )}
        </section>
    );
}

function PlayerVolume({ zone, onEq }: { zone: ZoneState; onEq: () => void }) {
    const t = useTranslations();
    return (
        <div className="current-volume">
            <VolumeSlider
                label={t("current.volumeFor", { name: zone.name })}
                volume={zone.volume}
                muted={zone.muted}
                onVolumeChange={(v) => {
                    api.zones.setVolume(zone.index, v).catch(logApiError);
                }}
                onMuteToggle={() => {
                    api.zones.toggleMute(zone.index).catch(logApiError);
                }}
                onUnmute={() => {
                    api.zones.setMute(zone.index, false).catch(logApiError);
                }}
            />
            <button
                className="current-text-button"
                onClick={onEq}
                aria-label={t("eq.title", { zone: zone.name })}
            >
                EQ{zone.eqEnabled ? " •" : ""}
            </button>
        </div>
    );
}
