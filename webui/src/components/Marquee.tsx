"use client";

import { useRef, useState, useEffect, type ReactNode } from "react";

export function Marquee({ children, className = "" }: { children: ReactNode; className?: string }) {
  const outerRef = useRef<HTMLDivElement>(null);
  const innerRef = useRef<HTMLSpanElement>(null);
  const [offset, setOffset] = useState(0);

  useEffect(() => {
    const check = () => {
      if (outerRef.current && innerRef.current) {
        const diff = innerRef.current.scrollWidth - outerRef.current.clientWidth;
        setOffset(diff > 0 ? diff : 0);
      }
    };
    check();
    const obs = new ResizeObserver(check);
    if (outerRef.current) obs.observe(outerRef.current);
    if (innerRef.current) obs.observe(innerRef.current);
    return () => obs.disconnect();
  }, [children]);

  return (
    <div ref={outerRef} className={`overflow-hidden whitespace-nowrap ${offset > 0 ? "" : "truncate"} ${className}`}>
      <span
        ref={innerRef}
        className={offset > 0 ? "inline-block animate-marquee" : "inline-block"}
        style={offset > 0 ? { "--marquee-offset": `-${offset}px` } as React.CSSProperties : undefined}
      >
        {children}
      </span>
    </div>
  );
}

