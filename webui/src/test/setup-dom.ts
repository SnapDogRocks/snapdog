import { afterEach, vi } from "vitest";
import { cleanup } from "@testing-library/react";

// jsdom lacks the layout and pointer-capture APIs Radix relies on. Assigned unconditionally:
// the types claim they exist, jsdom does not provide them.
const noop = vi.fn();
class ResizeObserverStub {
  observe = noop;
  unobserve = noop;
  disconnect = noop;
}
vi.stubGlobal("ResizeObserver", ResizeObserverStub);

const captured = new WeakMap<Element, Set<number>>();
Object.assign(Element.prototype, {
  setPointerCapture(this: Element, id: number) {
    const ids = captured.get(this) ?? new Set<number>();
    ids.add(id);
    captured.set(this, ids);
  },
  releasePointerCapture(this: Element, id: number) {
    captured.get(this)?.delete(id);
  },
  hasPointerCapture(this: Element, id: number) {
    return captured.get(this)?.has(id) ?? false;
  },
  scrollIntoView: noop,
});

afterEach(() => {
  cleanup();
});
