import { describe, expect, it } from "vitest";
import { scrollRowIntoList } from "./scroll-into-list";

function box(top: number, height: number) {
  return { getBoundingClientRect: () => ({ top, height }) } as unknown as HTMLElement;
}

function list(scrollTop: number, clientHeight: number) {
  return {
    scrollTop,
    clientHeight,
    getBoundingClientRect: () => ({ top: 100, height: clientHeight }),
  } as unknown as HTMLElement;
}

describe("scrollRowIntoList", () => {
  it("leaves a visible row alone", () => {
    const l = list(0, 200);
    expect(scrollRowIntoList(l, box(150, 40))).toBe(false);
    expect(l.scrollTop).toBe(0);
  });

  it("aligns a row below the view with the bottom edge", () => {
    const l = list(0, 200);
    // Row at 400 px inside the list, 40 px high.
    expect(scrollRowIntoList(l, box(500, 40))).toBe(true);
    expect(l.scrollTop).toBe(240);
  });

  it("aligns a row above the view with the top edge", () => {
    const l = list(300, 200);
    // Row at 50 px inside the list: rect top is 100 + 50 - 300.
    expect(scrollRowIntoList(l, box(-150, 40))).toBe(true);
    expect(l.scrollTop).toBe(50);
  });
});
