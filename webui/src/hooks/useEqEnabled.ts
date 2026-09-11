import { useState, useEffect } from "react";
import { api } from "@/lib/api";
import { logApiError } from "@/lib/log-api-error";

export function useEqEnabled(target: { zoneId?: number; clientId?: number }): [boolean, (v: boolean) => void] {
  const [enabled, setEnabled] = useState(false);
  useEffect(() => {
    const id = target.clientId ?? target.zoneId;
    if (!id) return;
    const fetcher = target.clientId ? api.clientEq.get : api.eq.get;
    fetcher(id).then((c) => { setEnabled(c.enabled); }).catch(logApiError);
  }, [target.zoneId, target.clientId]);
  return [enabled, setEnabled];
}
