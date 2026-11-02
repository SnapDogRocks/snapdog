import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useAppStore } from "./useAppStore";

describe("playlist list reload trigger", () => {
  beforeEach(() => {
    vi.stubGlobal("fetch", vi.fn(() => Promise.resolve(new Response("[]", { status: 200 }))));
    useAppStore.setState({ isConnected: false, playlistsVersion: 0 });
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("bumps the version on playlists_changed", () => {
    useAppStore.getState().bumpPlaylists();
    expect(useAppStore.getState().playlistsVersion).toBe(1);
  });

  it("bumps the version after a (re)connect, not on repeated connected updates", () => {
    useAppStore.getState().setConnected(true);
    expect(useAppStore.getState().playlistsVersion).toBe(1);
    useAppStore.getState().setConnected(true);
    expect(useAppStore.getState().playlistsVersion).toBe(1);
    useAppStore.getState().setConnected(false);
    useAppStore.getState().setConnected(true);
    expect(useAppStore.getState().playlistsVersion).toBe(2);
  });
});
