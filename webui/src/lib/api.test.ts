import { afterEach, beforeEach, describe, expect, it, vi, type Mock } from "vitest";
import { ApiError, api } from "./api";

const authMocks = vi.hoisted(() => ({
  getApiKey: vi.fn(() => "test-key"),
  clearApiKey: vi.fn(),
}));

vi.mock("./auth", () => authMocks);

describe("API authentication failures", () => {
  let unauthorizedEvents: number[];
  let reload: () => void;
  let reloadMock: Mock<() => void>;

  beforeEach(() => {
    unauthorizedEvents = [];
    reloadMock = vi.fn<() => void>();
    reload = () => { reloadMock(); };
    const browser = new EventTarget() as EventTarget & { location: { reload: () => void } };
    browser.location = { reload };
    browser.addEventListener("snapdog:unauthorized", () => unauthorizedEvents.push(1));
    vi.stubGlobal("window", browser);
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(new Response(null, { status: 401 })));
    authMocks.getApiKey.mockReturnValue("test-key");
    authMocks.clearApiKey.mockClear();
  });

  afterEach(() => vi.unstubAllGlobals());

  it("emits an unauthorized event, clears credentials, and rejects without reloading", async () => {
    const error = await api.health.check().catch((e: unknown) => e);

    expect(error).toBeInstanceOf(ApiError);
    expect(error).toMatchObject({ status: 401 });
    expect(authMocks.clearApiKey).toHaveBeenCalledOnce();
    expect(unauthorizedEvents).toHaveLength(1);
    expect(reloadMock).not.toHaveBeenCalled();
  });
});
