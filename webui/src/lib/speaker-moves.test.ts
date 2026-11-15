import { describe, expect, it, vi } from "vitest";
import { moveSpeakers, undoSpeakerMoves } from "./speaker-moves";
function setup() {
    const assignments = new Map([
        [1, 1],
        [2, 1],
        [3, 2],
    ]);
    const getZone = vi.fn((id: number) => Promise.resolve(assignments.get(id) ?? 0));
    const setZone = vi.fn((id: number, zone: number) => {
        assignments.set(id, zone);
        return Promise.resolve();
    });
    return { assignments, transport: { getZone, setZone, confirmed: vi.fn() } };
}
describe("speaker assignment batches", () => {
    it("deduplicates a selection and only records acknowledged changes, excluding no-ops", async () => {
        const { transport } = setup();
        transport.setZone.mockImplementation((id) =>
            id === 2 ? Promise.reject(new Error("offline")) : Promise.resolve(),
        );
        const result = await moveSpeakers([1, 1, 2, 3], 2, transport);
        expect(result).toEqual({ moved: [{ id: 1, from: 1, to: 2 }], failed: [2], skipped: [3] });
        expect(transport.confirmed).toHaveBeenCalledTimes(1);
        expect(transport.setZone.mock.calls).toEqual([
            [1, 2],
            [2, 2],
        ]);
    });
    it("restores original assignments and preserves a subsequent external move", async () => {
        const { assignments, transport } = setup();
        const result = await moveSpeakers([1, 2], 2, transport);
        assignments.set(2, 3);
        const undo = await undoSpeakerMoves(result.moved, transport);
        expect(assignments.get(1)).toBe(1);
        expect(assignments.get(2)).toBe(3);
        expect(undo.skipped).toEqual([2]);
    });
    it("reports a failed preflight without sending a write", async () => {
        const { transport } = setup();
        transport.getZone.mockRejectedValue(new Error("disconnected"));
        expect((await moveSpeakers([1], 2, transport)).failed).toEqual([1]);
        expect(transport.setZone).not.toHaveBeenCalled();
    });
    it("retains failed undo entries so the caller can retry only those", async () => {
        const { transport } = setup();
        const result = await moveSpeakers([1, 2], 2, transport);
        transport.setZone.mockRejectedValueOnce(new Error("unavailable"));
        const undo = await undoSpeakerMoves(result.moved, transport);
        expect(undo.failed).toEqual([1]);
        expect(undo.moved).toEqual([{ id: 2, from: 2, to: 1 }]);
    });
});
