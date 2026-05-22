"use client";

import { useEffect, useState } from "react";
import { HugeiconsIcon } from "@hugeicons/react";
import { Sun01Icon, Moon01Icon } from "@hugeicons/core-free-icons";
import { motion, AnimatePresence } from "framer-motion";

type Theme = "light" | "dark";

export function ThemeToggle() {
  const [mounted, setMounted] = useState(false);
  const [theme, setTheme] = useState<Theme>("dark"); // Safe default for SSR

  // Safely resolve active theme and mounted status asynchronously on mount to prevent Next.js hydration warnings
  useEffect(() => {
    const timer = setTimeout(() => {
      setMounted(true);
      const stored = localStorage.getItem("theme") as Theme | null;
      if (stored === "dark" || stored === "light") {
        setTheme(stored);
      } else {
        // Default to system preference if not set
        const systemDark = window.matchMedia("(prefers-color-scheme: dark)").matches;
        setTheme(systemDark ? "dark" : "light");
      }
    }, 0);

    return () => clearTimeout(timer);
  }, []);

  // Update root classes and localStorage when the theme changes
  useEffect(() => {
    if (!mounted) return;
    const root = document.documentElement;
    root.classList.remove("dark", "light");
    root.classList.add(theme);
    root.style.colorScheme = theme;
    localStorage.setItem("theme", theme);
  }, [theme, mounted]);

  const toggle = () => {
    setTheme((prev) => (prev === "dark" ? "light" : "dark"));
  };

  // Render a stable placeholder with matching dimensions during SSR to prevent layout shifts
  if (!mounted) {
    return <div className="w-8 h-8 rounded-md" aria-hidden="true" />;
  }

  return (
    <button
      onClick={toggle}
      className="p-1 rounded-md text-muted-foreground hover:text-foreground hover:bg-muted/50 transition-colors w-8 h-8 flex items-center justify-center cursor-pointer relative overflow-hidden"
      aria-label={theme === "dark" ? "Switch to light theme" : "Switch to dark theme"}
      title={theme === "dark" ? "Switch to light theme" : "Switch to dark theme"}
    >
      <AnimatePresence mode="wait" initial={false}>
        <motion.div
          key={theme}
          initial={{ y: -8, opacity: 0, rotate: -45 }}
          animate={{ y: 0, opacity: 1, rotate: 0 }}
          exit={{ y: 8, opacity: 0, rotate: 45 }}
          transition={{ duration: 0.15, ease: "easeInOut" }}
          className="flex items-center justify-center size-full"
        >
          <HugeiconsIcon icon={theme === "dark" ? Sun01Icon : Moon01Icon} size={16} />
        </motion.div>
      </AnimatePresence>
    </button>
  );
}
