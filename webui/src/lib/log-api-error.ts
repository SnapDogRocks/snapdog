export function logApiError(e: unknown): void {
  console.error("API error", e);
  if (typeof window === "undefined") return;

  const message = e instanceof Error
    ? e.message
    : typeof e === "string"
      ? e
      : e !== null && typeof e === "object" && "message" in e && typeof e.message === "string"
        ? e.message
        : String(e);

  window.dispatchEvent(new CustomEvent("snapdog:api-error", { detail: { message } }));
}
