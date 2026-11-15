"use client";

import { useEffect, useRef } from "react";

/**
 * Trap focus within a container element (WCAG 2.4.3).
 * Returns a ref to attach to the modal/dialog container.
 */
export function useFocusTrap<T extends HTMLElement>() {
  const ref = useRef<T>(null);

  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const previouslyFocused = document.activeElement instanceof HTMLElement
      ? document.activeElement
      : null;
    const hadTabIndex = el.hasAttribute("tabindex");
    const originalTabIndex = el.getAttribute("tabindex");
    let initialFocusComplete = false;

    const focusable = () => Array.from(
      el.querySelectorAll<HTMLElement>(
        'button, [href], input, select, textarea, summary, [contenteditable="true"], [tabindex]:not([tabindex="-1"])'
      ),
    ).filter((node) => {
      if (node.matches(":disabled, [aria-disabled='true'], [aria-hidden='true']")) return false;
      if (node.closest("[hidden], [inert], [aria-hidden='true']")) return false;
      const style = window.getComputedStyle(node);
      return style.display !== "none" && style.visibility !== "hidden" && node.getClientRects().length > 0;
    });

    const focusInitial = () => {
      if (initialFocusComplete) return;
      const first = focusable()[0];
      if (first) {
        first.focus();
        initialFocusComplete = true;
        return;
      }
      // Keep focus inside while asynchronous content mounts, then move it to
      // the first available control when the subtree changes.
      if (document.activeElement === previouslyFocused || document.activeElement === document.body) {
        el.tabIndex = -1;
        el.focus();
      }
    };

    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key !== "Tab") return;
      const nodes = focusable();
      const first = nodes[0];
      const last = nodes[nodes.length - 1];
      if (!first || !last) {
        e.preventDefault();
        el.focus();
      } else if (!el.contains(document.activeElement)) {
        e.preventDefault();
        (e.shiftKey ? last : first).focus();
      } else if (e.shiftKey && document.activeElement === first) {
        e.preventDefault();
        last.focus();
      } else if (!e.shiftKey && document.activeElement === last) {
        e.preventDefault();
        first.focus();
      }
    };

    focusInitial();
    const observer = new MutationObserver(focusInitial);
    observer.observe(el, {
      childList: true,
      subtree: true,
      attributes: true,
      attributeFilter: ["class", "style", "disabled", "hidden", "tabindex", "aria-hidden"],
    });
    el.addEventListener("keydown", handleKeyDown);
    return () => {
      observer.disconnect();
      el.removeEventListener("keydown", handleKeyDown);
      if (!hadTabIndex) el.removeAttribute("tabindex");
      else if (originalTabIndex !== null) el.setAttribute("tabindex", originalTabIndex);
      if (previouslyFocused?.isConnected && (el.contains(document.activeElement) || document.activeElement === el || document.activeElement === document.body)) {
        previouslyFocused.focus();
      }
    };
  }, []);

  return ref;
}
