// Shared drawing environment: palette, font cache, text measurement and path helpers.
import type { Settings } from '../shared/types';
import { clamp } from './anim';
import { parseColor } from './color';

export const FONT_STACK =
  '-apple-system, BlinkMacSystemFont, "SF Pro Text", "Segoe UI", Roboto, "Helvetica Neue", Arial, "Apple Color Emoji", "Segoe UI Emoji", sans-serif';
export const MONO_STACK = 'ui-monospace, "SF Mono", "Cascadia Mono", Consolas, Menlo, monospace';

const fontCache = new Map<number, string>();

/** CSS font string, cached by (weight, size rounded to 0.25px). */
export function font(weight: number, px: number, mono = false): string {
  const q = Math.max(1, Math.round(px * 4));
  const key = (mono ? 1e9 : 0) + weight * 1e5 + q;
  let f = fontCache.get(key);
  if (f === undefined) {
    f = `${weight} ${q / 4}px ${mono ? MONO_STACK : FONT_STACK}`;
    if (fontCache.size > 512) fontCache.clear();
    fontCache.set(key, f);
  }
  return f;
}

export interface Palette {
  dark: boolean;
  /** Card background (packed rgb). */
  bg: number;
  bgAlpha: number;
  border: number;
  text: number;
  muted: number;
  faint: number;
  accent: number;
}

export function makePalette(theme: Settings['theme'], systemDark: boolean): Palette {
  const dark = theme.mode === 'system' ? systemDark : theme.mode === 'dark';
  const accent = parseColor(theme.accent);
  return dark
    ? { dark, bg: 0x0f151f, bgAlpha: 0.93, border: 0xffffff, text: 0xf1f5fb, muted: 0xa3b0c2, faint: 0x6f7d92, accent }
    : { dark, bg: 0xfbfcfe, bgAlpha: 0.95, border: 0x0f151f, text: 0x101722, muted: 0x4f5d72, faint: 0x8793a6, accent };
}

export interface Env {
  ctx: CanvasRenderingContext2D;
  dpr: number;
  W: number;
  H: number;
  pal: Palette;
  /** Seconds, advances each frame; drives shimmer only. */
  t: number;
}

const widthCache = new Map<string, Map<string, number>>();
let widthEntries = 0;

/** Cached text width for the font string `f` (sets ctx.font as a side effect). */
export function textWidth(ctx: CanvasRenderingContext2D, f: string, s: string): number {
  let m = widthCache.get(f);
  if (m === undefined) {
    m = new Map();
    widthCache.set(f, m);
  }
  let w = m.get(s);
  if (w === undefined) {
    if (ctx.font !== f) ctx.font = f;
    w = ctx.measureText(s).width;
    if (++widthEntries > 6000) {
      widthCache.clear();
      widthEntries = 0;
      m = new Map();
      widthCache.set(f, m);
    }
    m.set(s, w);
  }
  return w;
}

/** Truncate `s` with an ellipsis so it fits `maxW`. */
export function fitText(ctx: CanvasRenderingContext2D, f: string, s: string, maxW: number): string {
  if (textWidth(ctx, f, s) <= maxW) return s;
  let lo = 0;
  let hi = s.length;
  while (lo < hi) {
    const mid = (lo + hi + 1) >> 1;
    if (textWidth(ctx, f, s.slice(0, mid) + '…') <= maxW) lo = mid;
    else hi = mid - 1;
  }
  return s.slice(0, Math.max(0, lo)).trimEnd() + '…';
}

/** Greedy word wrap into at most `maxLines` lines; the last line is ellipsised if needed. */
export function wrapText(ctx: CanvasRenderingContext2D, f: string, s: string, maxW: number, maxLines: number): string[] {
  const words = s.split(/\s+/).filter(Boolean);
  const lines: string[] = [];
  let cur = '';
  for (let i = 0; i < words.length; i++) {
    const next = cur ? cur + ' ' + words[i] : words[i];
    if (textWidth(ctx, f, next) <= maxW || !cur) {
      cur = next;
    } else {
      if (lines.length === maxLines - 1) {
        // Last allowed line: pull in all remaining words and truncate.
        cur = cur + ' ' + words.slice(i).join(' ');
        lines.push(fitText(ctx, f, cur, maxW));
        return lines;
      }
      lines.push(cur);
      cur = words[i];
    }
  }
  if (cur) lines.push(fitText(ctx, f, cur, maxW));
  return lines;
}

/** Rounded-rect path (works without ctx.roundRect). */
export function roundRect(ctx: CanvasRenderingContext2D, x: number, y: number, w: number, h: number, r: number): void {
  const rr = clamp(r, 0, Math.min(w, h) / 2);
  ctx.beginPath();
  ctx.moveTo(x + rr, y);
  ctx.arcTo(x + w, y, x + w, y + h, rr);
  ctx.arcTo(x + w, y + h, x, y + h, rr);
  ctx.arcTo(x, y + h, x, y, rr);
  ctx.arcTo(x, y, x + w, y, rr);
  ctx.closePath();
}

/** Snap a CSS-pixel coordinate to the device pixel grid (crisp text and hairlines). */
export const snap = (v: number, dpr: number): number => Math.round(v * dpr) / dpr;

export interface Rect { x: number; y: number; w: number; h: number }
