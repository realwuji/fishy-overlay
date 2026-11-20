// Demo-only backdrop: a chess.com-green board with pieces, drawn on its own canvas
// BEHIND the overlay canvas so the renderer can be previewed in a plain browser.
// It is never created inside the real overlay window.
import type { BoardGeometry } from '../shared/types';
import { displayCol, displayRow, fenBoard } from './chess';

const LIGHT = '#eeeed2';
const DARK = '#769656';
const GLYPH: Record<string, string> = {
  k: '♚︎', q: '♛︎', r: '♜︎', b: '♝︎', n: '♞︎', p: '♟︎',
};
const GLYPH_FONT = '"Segoe UI Symbol", "Apple Symbols", "Noto Sans Symbols 2", "DejaVu Sans", serif';

export function drawDemoBoard(
  canvas: HTMLCanvasElement, width: number, height: number, geometry: BoardGeometry, fen: string,
): void {
  const dpr = Math.max(1, window.devicePixelRatio || 1);
  canvas.width = Math.round(width * dpr);
  canvas.height = Math.round(height * dpr);
  canvas.style.width = `${width}px`;
  canvas.style.height = `${height}px`;
  const ctx = canvas.getContext('2d');
  if (!ctx) return;
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, width, height);

  const { x, y, size, flipped } = geometry;
  const s = size / 8;
  const board = fenBoard(fen);

  ctx.save();
  ctx.shadowColor = 'rgba(0,0,0,0.45)';
  ctx.shadowBlur = 30 * dpr;
  ctx.shadowOffsetY = 8 * dpr;
  ctx.fillStyle = DARK;
  ctx.fillRect(x, y, size, size);
  ctx.restore();

  for (let rank = 0; rank < 8; rank++) {
    for (let file = 0; file < 8; file++) {
      const col = displayCol(file, flipped);
      const row = displayRow(rank, flipped);
      const light = (file + rank) % 2 === 1;
      ctx.fillStyle = light ? LIGHT : DARK;
      ctx.fillRect(x + col * s, y + row * s, s + 0.5, s + 0.5);
    }
  }

  // Coordinates, chess.com style: tinted with the opposite square colour.
  ctx.font = `700 ${Math.max(9, s * 0.17)}px -apple-system, "Segoe UI", sans-serif`;
  ctx.textBaseline = 'top';
  for (let i = 0; i < 8; i++) {
    const rankDigit = flipped ? i + 1 : 8 - i;
    const rankLight = (rankDigit - 1 + (flipped ? 7 : 0)) % 2 === 1; // light squares have odd (file + rank)
    ctx.textAlign = 'left';
    ctx.fillStyle = rankLight ? DARK : LIGHT;
    ctx.fillText(String(rankDigit), x + 3, y + i * s + 3);
    const fileIdx = flipped ? 7 - i : i;
    const fileLight = (fileIdx + (flipped ? 7 : 0)) % 2 === 1;
    ctx.textAlign = 'right';
    ctx.textBaseline = 'bottom';
    ctx.fillStyle = fileLight ? DARK : LIGHT;
    ctx.fillText(String.fromCharCode(97 + fileIdx), x + (i + 1) * s - 3, y + size - 3);
    ctx.textBaseline = 'top';
  }

  // Pieces (solid glyphs; white pieces outlined dark so they read on both colours).
  ctx.textAlign = 'center';
  ctx.textBaseline = 'middle';
  ctx.font = `${s * 0.84}px ${GLYPH_FONT}`;
  ctx.lineJoin = 'round';
  ctx.lineWidth = Math.max(1.5, s * 0.035);
  for (let rank = 0; rank < 8; rank++) {
    for (let file = 0; file < 8; file++) {
      const ch = board[rank * 8 + file];
      if (ch === ' ') continue;
      const white = ch === ch.toUpperCase();
      const g = GLYPH[ch.toLowerCase()];
      const cx = x + (displayCol(file, flipped) + 0.5) * s;
      const cy = y + (displayRow(rank, flipped) + 0.54) * s;
      if (white) {
        ctx.strokeStyle = '#2b2b2b';
        ctx.strokeText(g, cx, cy);
        ctx.fillStyle = '#ffffff';
      } else {
        ctx.fillStyle = '#262522';
      }
      ctx.fillText(g, cx, cy);
    }
  }
}
