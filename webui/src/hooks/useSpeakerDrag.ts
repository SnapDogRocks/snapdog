"use client";
import { useEffect, useRef, useState, type PointerEvent as ReactPointerEvent } from "react";
interface Drag {
    ids: number[];
    x: number;
    y: number;
    target: number | null;
}
interface Gesture {
    pointer: number;
    startX: number;
    startY: number;
    ids: number[];
    active: boolean;
    element: HTMLElement;
}
export function useSpeakerDrag(onDrop: (ids: number[], zone: number) => void) {
    const gesture = useRef<Gesture | null>(null);
    const [drag, setDrag] = useState<Drag | null>(null);
    const cancel = (event?: ReactPointerEvent<HTMLButtonElement>) => {
        const g = gesture.current;
        if (event && g?.pointer !== event.pointerId) return;
        gesture.current = null;
        if (g?.element.hasPointerCapture(g.pointer)) g.element.releasePointerCapture(g.pointer);
        setDrag(null);
    };
    useEffect(() => {
        const escape = (e: KeyboardEvent) => {
            if (e.key === "Escape" && gesture.current) {
                e.preventDefault();
                cancel();
            }
        };
        window.addEventListener("keydown", escape);
        const onBlur = () => {
            cancel();
        };
        window.addEventListener("blur", onBlur);
        return () => {
            window.removeEventListener("keydown", escape);
            window.removeEventListener("blur", onBlur);
        };
    }, []);
    const targetAt = (x: number, y: number): number | null => {
        const target = document.elementFromPoint(x, y)?.closest<HTMLElement>("[data-drop-zone]");
        return target ? Number(target.dataset.dropZone) : null;
    };
    const start = (event: ReactPointerEvent<HTMLButtonElement>, ids: number[]) => {
        if (event.button !== 0 || !event.isPrimary) return;
        event.currentTarget.setPointerCapture(event.pointerId);
        gesture.current = {
            pointer: event.pointerId,
            startX: event.clientX,
            startY: event.clientY,
            ids,
            active: false,
            element: event.currentTarget,
        };
    };
    const move = (event: ReactPointerEvent<HTMLButtonElement>) => {
        const g = gesture.current;
        if (g?.pointer !== event.pointerId) return;
        if (!g.active && Math.hypot(event.clientX - g.startX, event.clientY - g.startY) < 6) return;
        g.active = true;
        const nav = document.querySelector<HTMLElement>(".current-zone-list");
        if (nav) {
            const rect = nav.getBoundingClientRect();
            if (
                event.clientY >= rect.top &&
                event.clientY <= rect.bottom &&
                event.clientX >= rect.left &&
                event.clientX <= rect.right
            ) {
                if (event.clientX > rect.right - 28) nav.scrollLeft += 15;
                if (event.clientX < rect.left + 28) nav.scrollLeft -= 15;
                if (event.clientY > rect.bottom - 28) nav.scrollTop += 15;
                if (event.clientY < rect.top + 28) nav.scrollTop -= 15;
            }
        }
        setDrag({
            ids: g.ids,
            x: event.clientX,
            y: event.clientY,
            target: targetAt(event.clientX, event.clientY),
        });
    };
    const end = (event: ReactPointerEvent<HTMLButtonElement>) => {
        const g = gesture.current;
        if (g?.pointer !== event.pointerId) return false;
        const target = g.active ? targetAt(event.clientX, event.clientY) : null;
        const tapped = !g.active;
        cancel();
        if (target !== null) onDrop(g.ids, target);
        return tapped;
    };
    return { drag, start, move, end, cancel };
}
