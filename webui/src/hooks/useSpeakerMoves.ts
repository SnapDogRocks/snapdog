"use client";
import { useEffect, useRef, useState } from "react";
import { api } from "@/lib/api";
import { useAppStore } from "@/stores/useAppStore";
import {
    moveSpeakers,
    undoSpeakerMoves,
    type MoveResult,
    type SpeakerMove,
} from "@/lib/speaker-moves";
const SUCCESS_NOTICE_MS = 5_000;

const transport = {
    getZone: api.clients.getZone,
    setZone: api.clients.setZone,
    confirmed: (move: SpeakerMove) => {
        const store = useAppStore.getState();
        const current = store.clients.get(move.id);
        if (current?.zone_index === move.from) store.updateClient(move.id, { zone_index: move.to });
    },
};
export function useSpeakerMoves() {
    const lock = useRef(false);
    const history = useRef<SpeakerMove[][]>([]);
    const [busy, setBusy] = useState(false);
    const [undoing, setUndoing] = useState(false);
    const [canUndo, setCanUndo] = useState(false);
    const [notice, setNotice] = useState<{
        result: MoveResult;
        undo: boolean;
    } | null>(null);
    useEffect(() => {
        // The history outlives its notification. Keep failures and skipped restores visible.
        if (
            !notice ||
            busy ||
            notice.result.failed.length > 0 ||
            (notice.undo && notice.result.skipped.length > 0)
        )
            return;
        const timer = setTimeout(() => {
            setNotice((current) => (current === notice ? null : current));
        }, SUCCESS_NOTICE_MS);
        return () => {
            clearTimeout(timer);
        };
    }, [notice, busy]);

    const move = async (ids: number[], to: number) => {
        if (lock.current || !ids.length || !useAppStore.getState().zones.has(to)) return;
        lock.current = true;
        setBusy(true);
        setUndoing(false);
        try {
            const result = await moveSpeakers(ids, to, transport);
            if (result.moved.length)
                history.current = [...history.current.slice(-19), result.moved];
            setNotice({ result, undo: false });
            setCanUndo(history.current.length > 0);
        } finally {
            lock.current = false;
            setBusy(false);
        }
    };
    const undo = async () => {
        if (lock.current) return;
        const batch = history.current.at(-1);
        if (!batch) return;
        lock.current = true;
        setBusy(true);
        setUndoing(true);
        try {
            const result = await undoSpeakerMoves(batch, transport);
            const retry = batch.filter((item) => result.failed.includes(item.id));
            history.current = [...history.current.slice(0, -1), ...(retry.length ? [retry] : [])];
            setNotice({ result, undo: true });
            setCanUndo(history.current.length > 0);
        } finally {
            lock.current = false;
            setBusy(false);
        }
    };
    return { move, undo, busy, undoing, canUndo, notice };
}
