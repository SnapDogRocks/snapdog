"use client";

import { useState, useEffect, useCallback, useMemo, useRef } from "react";
import { useTranslations } from "next-intl";
import { useFocusTrap } from "@/hooks/useFocusTrap";
import { Button } from "@/components/ui/button";
import { Slider } from "@/components/ui/slider";
import { api, type EqBand, type EqConfig } from "@/lib/api";
import { logApiError } from "@/lib/log-api-error";
import { computeResponse } from "@/lib/eq-response";
import {
  FILTER_TYPES,
  MAX_EQ_BANDS,
  FREQ_MIN_HZ,
  FREQ_MAX_HZ,
  GAIN_MIN_DB,
  GAIN_MAX_DB,
  GAIN_STEP_DB,
  Q_MIN,
  Q_MAX,
  Q_STEP,
  InteractiveEQCurve,
  FrequencyResponseCurve
} from "./EqCurve";

const PRESETS = ['flat', 'bass_boost', 'treble_boost', 'vocal', 'rock', 'jazz', 'classical', 'electronic', 'loudness', 'late_night'] as const;

const MAX_SPEAKER_RESULTS = 50;

const PRESET_LABELS: Record<string, string> = {
  flat: 'Flat',
  bass_boost: 'Bass Boost',
  treble_boost: 'Treble Boost',
  vocal: 'Vocal',
  rock: 'Rock',
  jazz: 'Jazz',
  classical: 'Classical',
  electronic: 'Electronic',
  loudness: 'Loudness',
  late_night: 'Late Night',
};

interface EqOverlayProps {
  zoneId?: number;
  clientId?: number;
  label: string;
  onClose: (enabled: boolean) => void;
}

const DEFAULT_BAND: EqBand = { freq: 1000, gain: 0, q: 1.0, type: "peaking" };

type Tab = "eq" | "speaker";

function limitEqBands(bands: EqBand[]): EqBand[] {
  return bands.slice(0, MAX_EQ_BANDS);
}

export function EqOverlay({ zoneId, clientId, label, onClose }: EqOverlayProps) {
  const t = useTranslations("eq");
  const trapRef = useFocusTrap<HTMLDivElement>();
  const [tab, setTab] = useState<Tab>("eq");
  const [config, setConfig] = useState<EqConfig>({ enabled: false, bands: [], preset: "flat" });
  const [abBypass, setAbBypass] = useState(false);
  const [selectedBand, setSelectedBand] = useState<number | null>(null);
  const [speakerMode, setSpeakerMode] = useState<"off" | "spinorama" | "custom">("off");
  const [speakerAbBypass, setSpeakerAbBypass] = useState(false);
  const [loading, setLoading] = useState(true);

  const showTabs = clientId != null;

  const eqApi = useMemo(() => clientId
    ? { get: () => api.clientEq.get(clientId), set: (c: EqConfig) => api.clientEq.set(clientId, c), applyPreset: (n: string) => api.clientEq.applyPreset(clientId, n) }
    : { get: () => api.eq.get(zoneId!), set: (c: EqConfig) => api.eq.set(zoneId!, c), applyPreset: (n: string) => api.eq.applyPreset(zoneId!, n) },
    [zoneId, clientId]);

  // Load current EQ
  useEffect(() => {
    eqApi.get().then((c) => { setConfig(c); setLoading(false); }).catch(() => setLoading(false));
  }, [eqApi]);

  // Push to server
  const pushConfig = useCallback(
    (c: EqConfig) => { eqApi.set(c).catch(logApiError); },
    [eqApi],
  );

  const update = useCallback(
    (patch: Partial<EqConfig>) => {
      setConfig((prev) => {
        const next = { ...prev, ...patch, preset: null };
        pushConfig(next);
        return next;
      });
    },
    [pushConfig],
  );

  const updateBand = useCallback(
    (idx: number, patch: Partial<EqBand>) => {
      setConfig((prev) => {
        const bands = prev.bands.map((b, i) => (i === idx ? { ...b, ...patch } : b));
        const next = { ...prev, bands, preset: null };
        pushConfig(next);
        return next;
      });
    },
    [pushConfig],
  );

  const addBand = () => {
    if (config.bands.length >= MAX_EQ_BANDS) return;
    update({ bands: [...config.bands, { ...DEFAULT_BAND }] });
  };

  const removeBand = (idx: number) => {
    update({ bands: config.bands.filter((_, i) => i !== idx) });
  };

  const applyPreset = (name: string) => {
    eqApi.applyPreset(name).then(setConfig).catch(logApiError);
  };

  const toggleEnabled = (on: boolean) => {
    if (on) {
      eqApi.get().then((c) => {
        const next = { ...c, enabled: true };
        setConfig(next);
        pushConfig(next);
      }).catch(logApiError);
    } else {
      eqApi.set({ ...config, enabled: false }).catch(logApiError);
      setConfig((prev) => ({ ...prev, enabled: false }));
    }
  };

  const toggleAB = () => {
    const next = !abBypass;
    setAbBypass(next);
    pushConfig({ ...config, enabled: !next });
  };

  const handleClose = () => {
    if (abBypass) {
      pushConfig({ ...config, enabled: true });
    }
    onClose(config.enabled);
  };

  const response = useMemo(
    () => (config.bands.length > 0 ? computeResponse(config.bands) : []),
    [config.bands],
  );

  if (loading) return null;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center" role="dialog" aria-modal="true" aria-label={t("title", { zone: label })} onKeyDown={(e) => { if (e.key === "Escape") handleClose(); }} onDragStart={(e) => e.preventDefault()}>
      <div className="absolute inset-0 bg-background/80 backdrop-blur-sm" onClick={handleClose} role="presentation" />
      <div className="relative z-10 w-full max-w-2xl mx-4 max-h-[90vh] overflow-y-auto rounded-2xl border border-border bg-card p-6 shadow-xl space-y-5" ref={trapRef}>
        {/* Header */}
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-3">
            <h2 className="text-lg font-semibold">{t("title", { zone: label })}</h2>
            {tab === "eq" && (
              <div className="inline-flex rounded-lg bg-muted p-0.5" role="radiogroup" aria-label={t("toggle")}>
                <button role="radio" aria-checked={!config.enabled} className={`px-3 py-1 text-xs rounded-md transition-colors ${!config.enabled ? 'bg-background shadow-sm font-medium' : 'text-muted-foreground'}`} onClick={() => toggleEnabled(false)}>Off</button>
                <button role="radio" aria-checked={config.enabled} className={`px-3 py-1 text-xs rounded-md transition-colors ${config.enabled ? 'bg-background shadow-sm font-medium' : 'text-muted-foreground'}`} onClick={() => toggleEnabled(true)}>On</button>
              </div>
            )}
            {tab === "speaker" && (
              <div className="inline-flex rounded-lg bg-muted p-0.5" role="radiogroup" aria-label={t("toggle")}>
                <button role="radio" aria-checked={speakerMode === "off"} className={`px-3 py-1 text-xs rounded-md transition-colors ${speakerMode === "off" ? 'bg-background shadow-sm font-medium' : 'text-muted-foreground'}`} onClick={() => setSpeakerMode("off")}>Off</button>
                <button role="radio" aria-checked={speakerMode === "spinorama"} className={`px-3 py-1 text-xs rounded-md transition-colors ${speakerMode === "spinorama" ? 'bg-background shadow-sm font-medium' : 'text-muted-foreground'}`} onClick={() => setSpeakerMode("spinorama")}>Spinorama</button>
                <button role="radio" aria-checked={speakerMode === "custom"} className={`px-3 py-1 text-xs rounded-md transition-colors ${speakerMode === "custom" ? 'bg-background shadow-sm font-medium' : 'text-muted-foreground'}`} onClick={() => setSpeakerMode("custom")}>Custom</button>
              </div>
            )}
          </div>
          <div className="flex items-center gap-2">
            {tab === "eq" && (
              <Button variant="ghost" size="sm" onClick={toggleAB} disabled={!config.enabled} className={abBypass ? "text-orange-500 font-semibold" : "text-muted-foreground"} aria-pressed={abBypass}>
                A/B
              </Button>
            )}
            {tab === "speaker" && (
              <Button variant="ghost" size="sm" onClick={() => setSpeakerAbBypass(!speakerAbBypass)} disabled={speakerMode === "off"} className={speakerAbBypass ? "text-orange-500 font-semibold" : "text-muted-foreground"} aria-pressed={speakerAbBypass}>
                A/B
              </Button>
            )}
            <Button variant="ghost" size="sm" onClick={handleClose} aria-label={t("close")}>✕</Button>
          </div>
        </div>

        {/* Segmented control tabs — only for client overlays */}
        {showTabs && (
          <div className="inline-flex rounded-lg bg-muted p-0.5 w-full" role="tablist">
            <button role="tab" aria-selected={tab === "eq"} className={`flex-1 px-4 py-1.5 text-sm rounded-md transition-colors ${tab === "eq" ? 'bg-background shadow-sm font-medium' : 'text-muted-foreground'}`} onClick={() => setTab("eq")}>EQ</button>
            <button role="tab" aria-selected={tab === "speaker"} className={`flex-1 px-4 py-1.5 text-sm rounded-md transition-colors ${tab === "speaker" ? 'bg-background shadow-sm font-medium' : 'text-muted-foreground'}`} onClick={() => setTab("speaker")}>Speaker</button>
          </div>
        )}

        {/* Tab content — crossfade, both mounted to avoid flash */}
        <div className={`${tab === "eq" ? '' : 'hidden'}`}>
          <>
            {config.enabled ? (
              <div className={`space-y-5 ${abBypass ? 'opacity-50 pointer-events-none' : ''} transition-opacity duration-150`}>
                <InteractiveEQCurve
                  bands={config.bands}
                  response={response}
                  selectedBand={selectedBand}
                  onSelectBand={setSelectedBand}
                  onBandChange={(idx, patch) => updateBand(idx, patch)}
	                  onRemoveBand={(idx) => removeBand(idx)}
	                  onAddBand={(freq, gain) => {
	                    if (config.bands.length >= MAX_EQ_BANDS) return;
	                    const bands = [...config.bands, { ...DEFAULT_BAND, freq: Math.round(freq), gain: Math.round(gain * 2) / 2 }];
	                    update({ bands });
	                    setSelectedBand(bands.length - 1);
                  }}
                />
                <div className="flex gap-1.5 overflow-x-auto scrollbar-none py-1 -mx-1 px-1" role="radiogroup" aria-label={t("presets")}>
                  {PRESETS.map((p) => (
                    <button
                      key={p}
                      onClick={() => applyPreset(p)}
                      role="radio"
                      aria-checked={config.preset === p}
                      className={`shrink-0 px-3 py-1 text-xs rounded-full transition-colors ${
                        config.preset === p
                          ? 'bg-primary text-primary-foreground'
                          : 'bg-muted hover:bg-muted/80 text-foreground'
                      }`}
                    >
                      {PRESET_LABELS[p] || p}
                    </button>
                  ))}
                </div>
                <div className="space-y-3">
                  {config.bands.map((band, idx) => (
                    <BandRow
                      key={idx}
                      band={band}
                      index={idx}
                      onChange={(patch) => updateBand(idx, patch)}
                      onRemove={() => removeBand(idx)}
                    />
                  ))}
                </div>
                {config.bands.length < MAX_EQ_BANDS && (
                  <Button variant="ghost" size="sm" onClick={addBand} className="w-full">
                    {t("addBand")}
                  </Button>
                )}
              </div>
            ) : (
              <FrequencyResponseCurve response={[]} curveLabel={t("curve")} />
            )}
          </>
        </div>
        {showTabs && (
          <div className={`${tab === "speaker" ? '' : 'hidden'}`}>
            <SpeakerTab clientId={clientId!} mode={speakerMode} setMode={setSpeakerMode} abBypass={speakerAbBypass} />
          </div>
        )}
      </div>
    </div>
  );
}

// ── Speaker Tab ───────────────────────────────────────────────

function SpeakerTab({ clientId, mode, setMode, abBypass }: { clientId: number; mode: "off" | "spinorama" | "custom"; setMode: (v: "off" | "spinorama" | "custom") => void; abBypass: boolean }) {
  const t = useTranslations("eq");
  const [search, setSearch] = useState("");
  const [speakers, setSpeakers] = useState<string[]>([]);
  const [currentConfig, setCurrentConfig] = useState<EqConfig | null>(null);
  const [appliedName, setAppliedName] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [customBands, setCustomBands] = useState<EqBand[]>([]);
  const [customSelected, setCustomSelected] = useState<number | null>(null);
  const [visibleCount, setVisibleCount] = useState(MAX_SPEAKER_RESULTS);
  const abBypassRef = useRef(false);
  const appliedNameRef = useRef<string | null>(null);

  useEffect(() => { abBypassRef.current = abBypass; }, [abBypass]);
  useEffect(() => { appliedNameRef.current = appliedName; }, [appliedName]);

  // Restore speaker EQ on unmount if A/B was active
  useEffect(() => () => {
    if (abBypassRef.current && appliedNameRef.current) {
      api.speakers.apply(clientId, appliedNameRef.current).catch(logApiError);
    }
  }, [clientId]);

  useEffect(() => {
    Promise.all([
      api.speakers.list(),
      api.speakers.get(clientId),
	    ]).then(([list, config]) => {
	      setSpeakers(list);
	      setCurrentConfig(config);
      const name = config.preset?.startsWith("spinorama:") ? config.preset.slice("spinorama:".length) : null;
      setAppliedName(name);
      setCustomBands(name ? [] : limitEqBands(config.bands));
      setVisibleCount(MAX_SPEAKER_RESULTS);
      if (config.enabled || name != null) setMode(name ? "spinorama" : "custom");
      setLoading(false);
    }).catch(() => setLoading(false));
  }, [clientId, setMode]);

  const filtered = useMemo(() => {
    if (!search) return speakers;
    const q = search.toLowerCase();
    return speakers.filter((s) => s.toLowerCase().includes(q));
  }, [speakers, search]);

  const visibleSpeakers = filtered.slice(0, visibleCount);

  const handleScroll = useCallback((e: React.UIEvent<HTMLDivElement>) => {
    const el = e.currentTarget;
    if (el.scrollTop + el.clientHeight >= el.scrollHeight - 40) {
      setVisibleCount((prev) => Math.min(prev + MAX_SPEAKER_RESULTS, filtered.length));
    }
  }, [filtered.length]);

  const applySpeaker = (name: string) => {
    api.speakers.apply(clientId, name).then((config) => {
      setCurrentConfig(config);
      setAppliedName(name);
    }).catch(logApiError);
  };

  const clearSpeaker = () => {
    api.speakers.apply(clientId, null).then((config) => {
      setCurrentConfig(config);
      setAppliedName(null);
    }).catch(logApiError);
  };

  const pushCustom = useCallback((bands: EqBand[]) => {
    const limitedBands = limitEqBands(bands);
    const eq: EqConfig = { enabled: limitedBands.length > 0, bands: limitedBands, preset: null };
    setCurrentConfig(eq);
    api.speakers.applyCustom(clientId, eq).catch(logApiError);
  }, [clientId]);

  const toggleEnabled = useCallback((on: boolean) => {
    if (!on) {
      api.speakers.apply(clientId, null).then((config) => {
        setCurrentConfig(config);
        setAppliedName(null);
      }).catch(logApiError);
    } else if (on && appliedName) {
      api.speakers.apply(clientId, appliedName).then((config) => {
        setCurrentConfig(config);
      }).catch(logApiError);
    } else if (customBands.length > 0) {
      pushCustom(customBands);
    }
  }, [appliedName, clientId, customBands, pushCustom]);

  // Sync parent mode with server
  const prevMode = useRef(mode);
  useEffect(() => {
    if (prevMode.current === mode) return;
    prevMode.current = mode;
    toggleEnabled(mode !== "off");
  }, [mode, toggleEnabled]);

  // React to A/B bypass changes from parent
  const prevAbBypass = useRef(abBypass);
  useEffect(() => {
    if (prevAbBypass.current === abBypass) return;
    prevAbBypass.current = abBypass;
    if (abBypass) {
      api.speakers.apply(clientId, null).catch(logApiError);
    } else if (appliedName) {
      api.speakers.apply(clientId, appliedName).catch(logApiError);
    }
  }, [abBypass, clientId, appliedName]);

  const response = useMemo(
    () => (currentConfig?.bands.length ? computeResponse(currentConfig.bands) : []),
    [currentConfig],
  );

  if (loading) return <div className="text-sm text-muted-foreground py-8 text-center">Loading speakers…</div>;

  return (
    <div className="space-y-5">
      {mode === "off" ? (
        <FrequencyResponseCurve response={[]} curveLabel="Speaker correction curve" />
      ) : mode === "custom" ? (
        <div className={`space-y-5 ${abBypass ? 'opacity-50 pointer-events-none' : ''} transition-opacity duration-150`}>
          <InteractiveEQCurve
            bands={customBands}
            response={computeResponse(customBands)}
            selectedBand={customSelected}
            onSelectBand={setCustomSelected}
            onBandChange={(idx, patch) => {
              const next = customBands.map((b, i) => i === idx ? { ...b, ...patch } : b);
              setCustomBands(next);
              pushCustom(next);
            }}
            onRemoveBand={(idx) => {
              const next = customBands.filter((_, i) => i !== idx);
              setCustomBands(next);
              setCustomSelected(null);
              pushCustom(next);
	            }}
	            onAddBand={(freq, gain) => {
	              if (customBands.length >= MAX_EQ_BANDS) return;
	              const next = [...customBands, { ...DEFAULT_BAND, freq: Math.round(freq), gain: Math.round(gain * 2) / 2 }];
	              setCustomBands(next);
	              setCustomSelected(next.length - 1);
	              pushCustom(next);
            }}
          />
          <div className="space-y-3">
            {customBands.map((band, idx) => (
              <BandRow
                key={idx}
                band={band}
                index={idx}
                onChange={(patch) => {
                  const next = customBands.map((b, i) => i === idx ? { ...b, ...patch } : b);
                  setCustomBands(next);
                  pushCustom(next);
                }}
                onRemove={() => {
                  const next = customBands.filter((_, i) => i !== idx);
                  setCustomBands(next);
                  if (customSelected === idx) setCustomSelected(null);
                  pushCustom(next);
                }}
              />
            ))}
          </div>
          {customBands.length < MAX_EQ_BANDS && (
            <Button
              variant="ghost"
              size="sm"
              onClick={() => {
                const next = [...customBands, { ...DEFAULT_BAND }];
                setCustomBands(next);
                pushCustom(next);
              }}
              className="w-full"
            >
              + {t("addBand")}
            </Button>
          )}
        </div>
      ) : (
        <div className={`space-y-5 ${abBypass ? 'opacity-50 pointer-events-none' : ''} transition-opacity duration-150`}>
        {/* Correction curve */}
        <FrequencyResponseCurve response={response} curveLabel="Speaker correction curve" />

      {/* Applied speaker + clear */}
      {appliedName && (
        <div className="flex items-center justify-between bg-muted/50 rounded-lg px-3 py-2">
          <span className="text-sm font-medium truncate">{appliedName}</span>
          <Button variant="ghost" size="sm" onClick={clearSpeaker} className="text-xs shrink-0">Clear</Button>
        </div>
      )}

      {/* Search */}
      <input
        type="text"
        value={search}
        onChange={(e) => {
          setSearch(e.target.value);
          setVisibleCount(MAX_SPEAKER_RESULTS);
        }}
        placeholder="Search speakers…"
        className="w-full bg-muted border border-border rounded-lg px-3 py-2 text-sm placeholder:text-muted-foreground focus:outline-none focus:ring-1 focus:ring-ring"
        aria-label="Search speakers"
      />

      {/* Results list */}
      <div className="max-h-48 overflow-y-auto rounded-lg border border-border divide-y divide-border" onScroll={handleScroll}>
        {visibleSpeakers.length === 0 ? (
          <div className="px-3 py-4 text-sm text-muted-foreground text-center">No speakers found</div>
        ) : (
          visibleSpeakers.map((name) => (
            <button
              key={name}
              onClick={() => applySpeaker(name)}
              className={`w-full text-left px-3 py-2 text-sm hover:bg-muted/50 transition-colors ${name === appliedName ? 'bg-primary/10 font-medium' : ''}`}
            >
              {name}
            </button>
          ))
        )}
      </div>
      </div>
      )}
    </div>
  );
}

// ── Band Row ──────────────────────────────────────────────────

function BandRow({
  band,
  index,
  onChange,
  onRemove,
}: {
  band: EqBand;
  index: number;
  onChange: (patch: Partial<EqBand>) => void;
  onRemove: () => void;
}) {
  const t = useTranslations("eq");
  return (
    <div className="flex items-center gap-2 text-sm">
      <span className="w-5 text-muted-foreground text-xs">{index + 1}</span>
      <select
        value={band.type}
        onChange={(e) => onChange({ type: e.target.value as EqBand["type"] })}
        className="bg-muted border border-border rounded px-1.5 py-1 text-xs w-20"
        aria-label={t("filterType", { n: index + 1 })}
      >
        {FILTER_TYPES.map((t) => (
          <option key={t} value={t}>{t.replace("_", " ")}</option>
        ))}
      </select>
      <div className="flex-1 space-y-0.5">
        <div className="flex items-center gap-2">
          <span className="w-8 text-xs text-muted-foreground">Hz</span>
          <Slider
            value={[Math.log10(band.freq)]}
            min={Math.log10(FREQ_MIN_HZ)}
            max={Math.log10(FREQ_MAX_HZ)}
            step={0.01}
            onValueChange={([v]) => onChange({ freq: Math.round(Math.pow(10, v)) })}
            className="flex-1"
            aria-label={t("frequency")}
          />
          <span className="w-12 text-xs tabular-nums text-right">{band.freq >= 1000 ? `${(band.freq / 1000).toFixed(1)}k` : band.freq}</span>
        </div>
        <div className="flex items-center gap-2">
          <span className="w-8 text-xs text-muted-foreground">dB</span>
          <Slider
            value={[band.gain]}
            min={GAIN_MIN_DB}
            max={GAIN_MAX_DB}
            step={GAIN_STEP_DB}
            onValueChange={([v]) => onChange({ gain: v })}
            className="flex-1"
            aria-label={t("gain")}
          />
          <span className="w-12 text-xs tabular-nums text-right">{band.gain > 0 ? "+" : ""}{band.gain.toFixed(1)}</span>
        </div>
        <div className="flex items-center gap-2">
          <span className="w-8 text-xs text-muted-foreground">Q</span>
          <Slider
            value={[band.q]}
            min={Q_MIN}
            max={Q_MAX}
            step={Q_STEP}
            onValueChange={([v]) => onChange({ q: Math.round(v * 10) / 10 })}
            className="flex-1"
            aria-label={t("qFactor")}
          />
          <span className="w-12 text-xs tabular-nums text-right">{band.q.toFixed(1)}</span>
        </div>
      </div>
      <Button variant="ghost" size="sm" onClick={onRemove} className="text-muted-foreground px-1" aria-label={t("removeBand")}>✕</Button>
    </div>
  );
}
