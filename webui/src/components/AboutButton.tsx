"use client";

import { useState, useEffect } from "react";
import { useFocusTrap } from "@/hooks/useFocusTrap";
import { api } from "@/lib/api";

export function AboutButton() {
  const [open, setOpen] = useState(false);

  return (
    <>
      <button
        onClick={() => setOpen(true)}
        className="p-1 rounded-md text-muted-foreground hover:text-foreground hover:bg-muted/50 transition-colors"
        aria-label="About"
      >
        <InfoIcon size={16} />
      </button>
      {open && <AboutOverlay onClose={() => setOpen(false)} />}
    </>
  );
}

function AboutOverlay({ onClose }: { onClose: () => void }) {
  const [version, setVersion] = useState<string | null>(null);
  const trapRef = useFocusTrap<HTMLDivElement>();

  useEffect(() => {
    api.system.version().then((v) => setVersion(v.version)).catch(() => {});
  }, []);

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center" role="dialog" aria-modal="true" aria-label="About SnapDog" onKeyDown={(e) => { if (e.key === "Escape") onClose(); }}>
      <div className="absolute inset-0 bg-background/80 backdrop-blur-sm" onClick={onClose} role="presentation" />
      <div ref={trapRef} className="relative z-10 w-full max-w-sm mx-4 rounded-2xl border border-border bg-card p-6 shadow-xl space-y-4 text-center">
        <img src="/assets/snapdog-icon.svg" alt="SnapDog" className="size-16 mx-auto opacity-80" />
        <h2 className="text-lg font-semibold">SnapDog</h2>
        <p className="text-sm text-muted-foreground">Multi-zone audio system with smart home integration</p>
        {version && (
          <p className="text-xs text-muted-foreground tabular-nums">v{version}</p>
        )}
        <div className="flex items-center justify-center gap-4 pt-2">
          <a
            href="https://github.com/metaneutrons/snapdog"
            target="_blank"
            rel="noopener noreferrer"
            className="flex items-center gap-1.5 text-sm text-muted-foreground hover:text-foreground transition-colors"
          >
            <GitHubIcon size={16} />
            GitHub
          </a>
        </div>
        <p className="text-xs text-muted-foreground">
          Licensed under{" "}
          <a
            href="https://www.gnu.org/licenses/gpl-3.0.html"
            target="_blank"
            rel="noopener noreferrer"
            className="underline hover:text-foreground transition-colors"
          >
            GPL-3.0
          </a>
        </p>
        <p className="text-[10px] text-muted-foreground/50">© 2026 Fabian Schmieder</p>
        <button onClick={onClose} className="mt-2 text-xs text-muted-foreground hover:text-foreground transition-colors">Close</button>
      </div>
    </div>
  );
}

function InfoIcon({ size = 16 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={1.5} strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <circle cx="12" cy="12" r="10" />
      <path d="M12 16v-4" />
      <path d="M12 8h.01" />
    </svg>
  );
}

function GitHubIcon({ size = 16 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">
      <path d="M12 0C5.37 0 0 5.37 0 12c0 5.31 3.435 9.795 8.205 11.385.6.105.825-.255.825-.57 0-.285-.015-1.23-.015-2.235-3.015.555-3.795-.735-4.035-1.41-.135-.345-.72-1.41-1.23-1.695-.42-.225-1.02-.78-.015-.795.945-.015 1.62.87 1.845 1.23 1.08 1.815 2.805 1.305 3.495.99.105-.78.42-1.305.765-1.605-2.67-.3-5.46-1.335-5.46-5.925 0-1.305.465-2.385 1.23-3.225-.12-.3-.54-1.53.12-3.18 0 0 1.005-.315 3.3 1.23.96-.27 1.98-.405 3-.405s2.04.135 3 .405c2.295-1.56 3.3-1.23 3.3-1.23.66 1.65.24 2.88.12 3.18.765.84 1.23 1.905 1.23 3.225 0 4.605-2.805 5.625-5.475 5.925.435.375.81 1.095.81 2.22 0 1.605-.015 2.895-.015 3.3 0 .315.225.69.825.57A12.02 12.02 0 0024 12c0-6.63-5.37-12-12-12z" />
    </svg>
  );
}
