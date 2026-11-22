import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, screen } from "@testing-library/react";
import type { ZoneState } from "@/stores/useAppStore";
import { PlaylistBrowser } from "./PlaylistBrowser";
import { renderWithIntl } from "@/test/render";

const mocks = vi.hoisted(() => ({
  scroll: vi.fn(() => true),
  tracksById: vi.fn((): Promise<unknown[]> => Promise.resolve(
    ["a", "b", "c"].map((id) => ({ id, title: id, artist: "X", duration: 200, cover_art: null })),
  )),
}));

vi.mock("@/lib/scroll-into-list", () => ({ scrollRowIntoList: mocks.scroll }));
vi.mock("@/lib/api", () => ({
  api: {
    media: {
      playlists: vi.fn(() =>
        Promise.resolve([
          { index: 0, playlist_id: "subsonic:1", name: "Mix", song_count: 3, duration: 600, cover_art: null },
        ]),
      ),
      tracksById: mocks.tracksById,
    },
    zones: { playPlaylistById: vi.fn(() => Promise.resolve()) },
  },
}));

function zone(trackIndex: number, volume = 50): ZoneState {
  return {
    index: 1,
    volume,
    track: { title: "t", artist: "a", playlist_id: "subsonic:1", playlist_track_index: trackIndex },
  } as unknown as ZoneState;
}

async function openPlaylist() {
  fireEvent.click(await screen.findByRole("button", { name: "Open Mix" }));
  await screen.findByText("c");
}

describe("PlaylistBrowser, current track", () => {
  beforeEach(() => { mocks.scroll.mockClear(); });

  it("scrolls once on open and not again on an unrelated re-render", async () => {
    const { rerender } = renderWithIntl(<PlaylistBrowser zone={zone(2)} />);
    await openPlaylist();
    expect(mocks.scroll).toHaveBeenCalledTimes(1);
    const [, row] = mocks.scroll.mock.calls[0] as unknown as [HTMLElement, HTMLElement];
    expect(row.getAttribute("data-track-index")).toBe("2");

    rerender(<PlaylistBrowser zone={zone(2, 70)} />);
    rerender(<PlaylistBrowser zone={zone(2, 30)} />);
    expect(mocks.scroll).toHaveBeenCalledTimes(1);
  });

  it("scrolls once when the track changes", async () => {
    const { rerender } = renderWithIntl(<PlaylistBrowser zone={zone(0)} />);
    await openPlaylist();
    mocks.scroll.mockClear();

    rerender(<PlaylistBrowser zone={zone(1)} />);
    rerender(<PlaylistBrowser zone={zone(1, 80)} />);
    expect(mocks.scroll).toHaveBeenCalledTimes(1);
    const [, row] = mocks.scroll.mock.calls[0] as unknown as [HTMLElement, HTMLElement];
    expect(row.getAttribute("data-track-index")).toBe("1");
  });
});

describe("PlaylistBrowser, accessibility and keys", () => {
  it("names the search field", async () => {
    renderWithIntl(<PlaylistBrowser zone={zone(0)} />);
    expect(await screen.findByRole("textbox", { name: "Search playlists…" })).toBeTruthy();
  });

  it("renders a playlist that holds the same track twice without a key warning", async () => {
    const errors = vi.spyOn(console, "error").mockImplementation(() => undefined);
    mocks.tracksById.mockResolvedValueOnce(
      ["a", "a", "c"].map((id, i) => ({ id, title: `${id}${i}`, artist: "X", duration: 200, cover_art: null })),
    );
    renderWithIntl(<PlaylistBrowser zone={zone(0)} />);
    fireEvent.click(await screen.findByRole("button", { name: "Open Mix" }));
    await screen.findByText("c2");
    expect(screen.getByText("a0")).toBeTruthy();
    expect(screen.getByText("a1")).toBeTruthy();
    const keyWarnings = errors.mock.calls.filter((call) => String(call[0]).includes("same key"));
    errors.mockRestore();
    expect(keyWarnings).toEqual([]);
  });
});
