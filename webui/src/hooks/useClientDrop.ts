"use client";

import { useState, useCallback, type DragEvent } from "react";
import { api } from "@/lib/api";
import { logApiError } from "@/lib/log-api-error";

export const MIME = "application/x-snapdog-client";

/** Shared drag-and-drop logic for dropping a client onto a zone. */
export function useClientDrop(zoneIndex: number) {
  const [dragOver, setDragOver] = useState(false);

  const onDragOver = useCallback((e: DragEvent) => {
    if (e.dataTransfer.types.includes(MIME)) {
      e.preventDefault();
      e.dataTransfer.dropEffect = "move";
      setDragOver(true);
    }
  }, []);

  const onDragLeave = useCallback(() => { setDragOver(false); }, []);

  const onDrop = useCallback(
    (e: DragEvent) => {
      e.preventDefault();
      setDragOver(false);
      const clientIndex = Number(e.dataTransfer.getData(MIME));
      if (!isNaN(clientIndex)) {
        api.clients.setZone(clientIndex, zoneIndex).catch(logApiError);
      }
    },
    [zoneIndex],
  );

  return { dragOver, dragHandlers: { onDragOver, onDragLeave, onDrop } };
}
