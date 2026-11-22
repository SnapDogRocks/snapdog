/**
 * Sends values one at a time, latest wins: while a request is in flight, newer values replace
 * each other and only the last one is sent when it returns. A drag that produces a hundred
 * values therefore costs two requests, the requests cannot overtake each other, and the server
 * ends up with the final value.
 *
 * `onSuccess` and `onError` run only for the request that settles with nothing newer queued,
 * so a reconcile never overwrites an edit that is still on its way.
 */
export interface LatestSender<T> {
  push: (value: T) => void;
  /** A request is in flight or queued. */
  busy: () => boolean;
}

export function createLatestSender<T, R>(
  send: (value: T) => Promise<R>,
  handlers: { onSuccess?: (result: R, sent: T) => void; onError?: (error: unknown, sent: T) => void },
): LatestSender<T> {
  let inFlight = false;
  let queued: { value: T } | null = null;

  const pump = () => {
    if (inFlight || !queued) return;
    const { value } = queued;
    queued = null;
    inFlight = true;
    send(value).then(
      (result) => {
        inFlight = false;
        if (!queued) handlers.onSuccess?.(result, value);
        pump();
      },
      (error: unknown) => {
        inFlight = false;
        if (!queued) handlers.onError?.(error, value);
        pump();
      },
    );
  };

  return {
    push: (value) => {
      queued = { value };
      pump();
    },
    busy: () => inFlight || queued !== null,
  };
}
