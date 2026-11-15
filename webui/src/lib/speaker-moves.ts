/** A batch is a set of acknowledged individual assignments, not a server transaction. */
export interface SpeakerMove {
    id: number;
    from: number;
    to: number;
}
export interface MoveResult {
    moved: SpeakerMove[];
    failed: number[];
    skipped: number[];
}
interface MoveTransport {
    getZone: (id: number) => Promise<number>;
    setZone: (id: number, zone: number) => Promise<void>;
    confirmed: (move: SpeakerMove) => void;
}
export async function moveSpeakers(
    ids: number[],
    to: number,
    transport: MoveTransport,
): Promise<MoveResult> {
    const result: MoveResult = { moved: [], failed: [], skipped: [] };
    for (const id of new Set(ids)) {
        try {
            const from = await transport.getZone(id);
            if (from === to) {
                result.skipped.push(id);
                continue;
            }
            await transport.setZone(id, to);
            const move = { id, from, to };
            result.moved.push(move);
            transport.confirmed(move);
        } catch {
            result.failed.push(id);
        }
    }
    return result;
}
export async function undoSpeakerMoves(
    moves: SpeakerMove[],
    transport: MoveTransport,
): Promise<MoveResult> {
    const result: MoveResult = { moved: [], failed: [], skipped: [] };
    for (const move of moves) {
        try {
            // Another controller may have moved this speaker since our command.
            if ((await transport.getZone(move.id)) !== move.to) {
                result.skipped.push(move.id);
                continue;
            }
            await transport.setZone(move.id, move.from);
            const restored = { id: move.id, from: move.to, to: move.from };
            result.moved.push(restored);
            transport.confirmed(restored);
        } catch {
            result.failed.push(move.id);
        }
    }
    return result;
}
