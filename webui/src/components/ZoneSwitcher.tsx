"use client";

import { useState } from "react";
import { motion, AnimatePresence, useReducedMotion, type PanInfo } from "framer-motion";
import type { ZoneState } from "@/stores/useAppStore";

const SWIPE_THRESHOLD = 50;
const SWIPE_VELOCITY = 500;

interface ZoneSwitcherProps {
  zones: ZoneState[];
  selectedIndex: number;
  onSelect: (index: number) => void;
  children: (zone: ZoneState) => React.ReactNode;
}

export function ZoneSwitcher({ zones, selectedIndex, onSelect, children }: ZoneSwitcherProps) {
  const [direction, setDirection] = useState(0);
  const reduced = useReducedMotion();

  if (zones.length === 0) return null;

  const currentIdx = zones.findIndex((z) => z.index === selectedIndex);
  const safeIdx = currentIdx >= 0 ? currentIdx : 0;
  const zone = zones[safeIdx];
  if (!zone) return null;

  function go(delta: number) {
    const next = safeIdx + delta;
    const nextZone = zones[next];
    if (!nextZone) return;
    setDirection(delta);
    onSelect(nextZone.index);
  }

  function handleDragEnd(_: unknown, info: PanInfo) {
    if (info.offset.x < -SWIPE_THRESHOLD || info.velocity.x < -SWIPE_VELOCITY) go(1);
    else if (info.offset.x > SWIPE_THRESHOLD || info.velocity.x > SWIPE_VELOCITY) go(-1);
  }

  return (
    <div className="flex flex-1 flex-col min-h-0">
      {/* Swipeable zone content */}
      <div className="relative flex-1 overflow-hidden">
        <AnimatePresence initial={false} custom={direction} mode="popLayout">
          <motion.div
            key={zone.index}
            custom={direction}
            initial={reduced ? false : { x: direction > 0 ? "100%" : "-100%", opacity: 0 }}
            animate={{ x: 0, opacity: 1 }}
            exit={reduced ? { opacity: 0 } : { x: direction > 0 ? "-100%" : "100%", opacity: 0 }}
            transition={reduced ? { duration: 0 } : { type: "spring", stiffness: 300, damping: 30 }}
            drag={reduced ? false : "x"}
            dragConstraints={{ left: 0, right: 0 }}
            dragElastic={0.2}
            onDragEnd={handleDragEnd}
            className="absolute inset-0 flex flex-col"
          >
            {children(zone)}
          </motion.div>
        </AnimatePresence>
      </div>

      {/* Dot indicators */}
      {zones.length > 1 && (
        <div className="flex justify-center gap-1.5 py-3">
          {zones.map((z, i) => (
            <button
              key={z.index}
              onClick={() => { setDirection(i > safeIdx ? 1 : -1); onSelect(z.index); }}
              className={`size-1.5 rounded-full transition-all ${
                i === safeIdx ? "bg-primary w-4" : "bg-muted-foreground/30"
              }`}
              aria-label={z.name}
              aria-current={i === safeIdx ? "true" : undefined}
            />
          ))}
        </div>
      )}
    </div>
  );
}
