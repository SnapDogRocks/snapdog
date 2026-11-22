"use client";

import { useRef } from "react";

export interface Choice<T extends string> {
  value: T;
  label: string;
}

/**
 * A radio group of segment buttons with one tab stop: the selected option is in the tab
 * order, the arrow keys (and Home and End) move the selection, as the ARIA radio group
 * pattern describes.
 */
export function SegmentedChoice<T extends string>({
  label,
  options,
  value,
  onChange,
  className = "",
}: {
  label: string;
  options: readonly Choice<T>[];
  value: T;
  onChange: (value: T) => void;
  className?: string;
}) {
  const refs = useRef<(HTMLButtonElement | null)[]>([]);
  const selected = Math.max(0, options.findIndex((o) => o.value === value));

  const move = (to: number) => {
    const index = (to + options.length) % options.length;
    const option = options[index];
    if (!option) return;
    onChange(option.value);
    refs.current[index]?.focus();
  };

  return (
    <div
      className={`inline-flex rounded-lg bg-muted p-0.5 ${className}`}
      role="radiogroup"
      aria-label={label}
      onKeyDown={(e) => {
        const step = { ArrowRight: 1, ArrowDown: 1, ArrowLeft: -1, ArrowUp: -1 }[e.key];
        if (step !== undefined) move(selected + step);
        else if (e.key === "Home") move(0);
        else if (e.key === "End") move(options.length - 1);
        else return;
        e.preventDefault();
      }}
    >
      {options.map((option, index) => {
        const checked = index === selected;
        return (
          <button
            key={option.value}
            ref={(el) => { refs.current[index] = el; }}
            type="button"
            role="radio"
            aria-checked={checked}
            tabIndex={checked ? 0 : -1}
            className={`px-3 py-1 text-xs rounded-md transition-colors ${checked ? "bg-background shadow-sm font-medium" : "text-muted-foreground"}`}
            onClick={() => { onChange(option.value); }}
          >
            {option.label}
          </button>
        );
      })}
    </div>
  );
}
