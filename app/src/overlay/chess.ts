// Chess-notation helpers shared by the renderer and the demo board.
import { MOVE_CLASSES, type MoveClass } from '../shared/types';

/** Parsed square: file/rank 0..7 (a1 = 0,0). Returns false for malformed input. */
export function parseSquare(sq: string, out: { file: number; rank: number }): boolean {
  if (typeof sq !== 'string' || sq.length < 2) return false;
  const f = sq.charCodeAt(0) - 97;
  const r = sq.charCodeAt(1) - 49;
  if (f < 0 || f > 7 || r < 0 || r > 7) return false;
  out.file = f;
  out.rank = r;
  return true;
}

/** Display column (0..7, left to right) for a file. */
export const displayCol = (file: number, flipped: boolean): number => (flipped ? 7 - file : file);
/** Display row (0..7, top to bottom) for a rank. */
export const displayRow = (rank: number, flipped: boolean): number => (flipped ? rank : 7 - rank);

export const isKnightMove = (ff: number, fr: number, tf: number, tr: number): boolean => {
  const a = Math.abs(ff - tf);
  const b = Math.abs(fr - tr);
  return (a === 1 && b === 2) || (a === 2 && b === 1);
};

const CLASS_ORDER: Record<MoveClass, number> = MOVE_CLASSES.reduce(
  (acc, c, i) => ((acc[c] = i), acc),
  {} as Record<MoveClass, number>,
);
export const classIndex = (c: MoveClass): number => CLASS_ORDER[c] ?? MOVE_CLASSES.length;

/** "+0.3", "-1.20", "M3", "-M2" (white point of view). */
export function formatEval(cp: number | null, mate: number | null, decimals = 1): string {
  if (mate !== null && mate !== undefined) return mate < 0 ? `-M${-mate}` : `M${mate}`;
  if (cp === null || cp === undefined) return '0.0';
  const v = cp / 100;
  const s = Math.abs(v).toFixed(decimals);
  if (Number(s) === 0) return (0).toFixed(decimals);
  return (v > 0 ? '+' : '-') + s;
}

/** Evaluation in pawns clamped to a sane display range, or null when unknown. */
export const evalPawns = (cp: number | null): number => (cp === null ? 0 : Math.max(-30, Math.min(30, cp / 100)));

/** "1. e4 e5 2. Nf3" style PV from a FEN's side to move and fullmove number. */
export function formatPv(fen: string, sans: string[], maxPlies = 8): string {
  const parts = fen.split(' ');
  let black = parts[1] === 'b';
  let move = parseInt(parts[5] ?? '1', 10);
  if (!Number.isFinite(move) || move < 1) move = 1;
  let out = '';
  const n = Math.min(sans.length, maxPlies);
  for (let i = 0; i < n; i++) {
    if (!black) out += `${move}. `;
    else if (i === 0) out += `${move}… `;
    out += sans[i];
    if (i < n - 1) out += ' ';
    if (black) move++;
    black = !black;
  }
  return out;
}

/** Piece placement of a FEN into 64 chars indexed [rank*8 + file], ' ' for empty. */
export function fenBoard(fen: string): string[] {
  const out = new Array<string>(64).fill(' ');
  const rows = fen.split(' ')[0].split('/');
  for (let i = 0; i < 8 && i < rows.length; i++) {
    const rank = 7 - i;
    let file = 0;
    for (const ch of rows[i]) {
      const d = ch.charCodeAt(0) - 48;
      if (d >= 1 && d <= 8) file += d;
      else if (file < 8) out[rank * 8 + file++] = ch;
    }
  }
  return out;
}
