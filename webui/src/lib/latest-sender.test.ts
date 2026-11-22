import { describe, expect, it, vi } from "vitest";
import { createLatestSender } from "./latest-sender";

function deferred() {
  let resolve: (v: number) => void = () => undefined;
  let reject: (e: unknown) => void = () => undefined;
  const promise = new Promise<number>((res, rej) => { resolve = res; reject = rej; });
  return { promise, resolve, reject };
}

describe("createLatestSender", () => {
  it("sends one request at a time and then only the latest value", async () => {
    const calls: { value: number; d: ReturnType<typeof deferred> }[] = [];
    const onSuccess = vi.fn();
    const sender = createLatestSender((value: number) => {
      const d = deferred();
      calls.push({ value, d });
      return d.promise;
    }, { onSuccess });

    for (let i = 1; i <= 100; i++) sender.push(i);
    expect(calls.map((c) => c.value)).toEqual([1]);

    calls[0]?.d.resolve(1);
    await Promise.resolve();
    await Promise.resolve();
    expect(calls.map((c) => c.value)).toEqual([1, 100]);
    expect(onSuccess).not.toHaveBeenCalled();

    calls[1]?.d.resolve(100);
    await Promise.resolve();
    await Promise.resolve();
    expect(onSuccess).toHaveBeenCalledExactlyOnceWith(100, 100);
    expect(sender.busy()).toBe(false);
  });

  it("reports an error only when nothing newer is queued", async () => {
    const onError = vi.fn();
    const d = deferred();
    const sender = createLatestSender(() => d.promise, { onError });
    sender.push(1);
    d.reject(new Error("boom"));
    await Promise.resolve();
    await Promise.resolve();
    expect(onError).toHaveBeenCalledOnce();
  });
});
