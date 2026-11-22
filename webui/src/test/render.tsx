import type { ReactElement } from "react";
import { render } from "@testing-library/react";
import { NextIntlClientProvider } from "next-intl";
import messages from "../../messages/en.json";

/** Render a component with the English messages, as the app does. */
export function renderWithIntl(ui: ReactElement) {
  const wrap = (node: ReactElement) => (
    <NextIntlClientProvider locale="en" messages={messages}>
      {node}
    </NextIntlClientProvider>
  );
  const result = render(wrap(ui));
  return { ...result, rerender: (next: ReactElement) => { result.rerender(wrap(next)); } };
}

/**
 * Give a Radix slider root a 100 px wide box, so a pointer at `clientX` maps to
 * `min + clientX / 100 * (max - min)`. jsdom has no layout.
 */
export function layoutSlider(root: Element) {
  const rect = new DOMRect(0, 0, 100, 10);
  root.getBoundingClientRect = () => rect;
}
