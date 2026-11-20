// Colour helpers with string caches so the render loop does not allocate a new
// `rgba(...)` string for every fill.
import { clamp } from './anim';

const parsed = new Map<string, number>();

/** Parse "#rgb", "#rrggbb" or "#rrggbbaa" into a packed 0xRRGGBB int (alpha ignored). */
export function parseColor(s: string): number {
  const hit = parsed.get(s);
  if (hit !== undefined) return hit;
  let v = 0x94a3b8;
  const m = /^#([0-9a-f]{3,8})$/i.exec(s.trim());
  if (m) {
    const h = m[1];
    if (h.length === 3 || h.length === 4) {
      v = parseInt(h[0] + h[0] + h[1] + h[1] + h[2] + h[2], 16);
    } else if (h.length === 6 || h.length === 8) {
      v = parseInt(h.slice(0, 6), 16);
    }
  }
  if (parsed.size > 512) parsed.clear();
  parsed.set(s, v);
  return v;
}

const strCache = new Map<number, string>();

/** `rgba()` string for a packed colour and an alpha in 0..1 (quantised to 1/100). */
export function rgba(rgb: number, alpha: number): string {
  const aq = Math.round(clamp(alpha, 0, 1) * 100);
  const key = rgb * 128 + aq;
  let s = strCache.get(key);
  if (s === undefined) {
    s = `rgba(${(rgb >> 16) & 255},${(rgb >> 8) & 255},${rgb & 255},${aq / 100})`;
    if (strCache.size > 4096) strCache.clear();
    strCache.set(key, s);
  }
  return s;
}

export const packRgb = (r: number, g: number, b: number): number =>
  ((clamp(Math.round(r), 0, 255) << 16) | (clamp(Math.round(g), 0, 255) << 8) | clamp(Math.round(b), 0, 255));

export const redOf = (c: number): number => (c >> 16) & 255;
export const greenOf = (c: number): number => (c >> 8) & 255;
export const blueOf = (c: number): number => c & 255;

export function mixRgb(a: number, b: number, t: number): number {
  return packRgb(
    redOf(a) + (redOf(b) - redOf(a)) * t,
    greenOf(a) + (greenOf(b) - greenOf(a)) * t,
    blueOf(a) + (blueOf(b) - blueOf(a)) * t,
  );
}

/** Relative luminance 0..1 (sRGB, approximate). */
export function luminance(c: number): number {
  return (0.2126 * redOf(c) + 0.7152 * greenOf(c) + 0.0722 * blueOf(c)) / 255;
}

/** Readable foreground (near-white or near-black) for text drawn on `bg`. */
export const onColor = (bg: number): number => (luminance(bg) > 0.6 ? 0x0c1320 : 0xffffff);

export function hexOf(c: number): string {
  return '#' + (c & 0xffffff).toString(16).padStart(6, '0');
}
