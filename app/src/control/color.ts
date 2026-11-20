// Colour helpers (hex <-> rgb <-> hsl, contrast).

export function normHex(v: string): string | null {
  const m = /^#?([0-9a-f]{3}|[0-9a-f]{6})$/i.exec(v.trim());
  if (!m) return null;
  let x = m[1].toLowerCase();
  if (x.length === 3) x = x.split('').map((c) => c + c).join('');
  return `#${x}`;
}

export function hexToRgb(hex: string): [number, number, number] {
  const n = parseInt((normHex(hex) ?? '#000000').slice(1), 16);
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}

export function rgbToHex(r: number, g: number, b: number): string {
  const t = (v: number) => Math.round(Math.min(255, Math.max(0, v))).toString(16).padStart(2, '0');
  return `#${t(r)}${t(g)}${t(b)}`;
}

export function hexToHsl(hex: string): [number, number, number] {
  const [r, g, b] = hexToRgb(hex).map((v) => v / 255);
  const max = Math.max(r, g, b), min = Math.min(r, g, b);
  const l = (max + min) / 2;
  const d = max - min;
  if (d === 0) return [0, 0, l];
  const s = d / (1 - Math.abs(2 * l - 1));
  let hh: number;
  if (max === r) hh = ((g - b) / d) % 6;
  else if (max === g) hh = (b - r) / d + 2;
  else hh = (r - g) / d + 4;
  return [(hh * 60 + 360) % 360, s, l];
}

export function hslToHex(h: number, s: number, l: number): string {
  const c = (1 - Math.abs(2 * l - 1)) * s;
  const x = c * (1 - Math.abs(((h / 60) % 2) - 1));
  const m = l - c / 2;
  const [r, g, b] = h < 60 ? [c, x, 0] : h < 120 ? [x, c, 0] : h < 180 ? [0, c, x] : h < 240 ? [0, x, c] : h < 300 ? [x, 0, c] : [c, 0, x];
  return rgbToHex((r + m) * 255, (g + m) * 255, (b + m) * 255);
}

export function luminance(hex: string): number {
  const f = (v: number) => { const c = v / 255; return c <= 0.03928 ? c / 12.92 : Math.pow((c + 0.055) / 1.055, 2.4); };
  const [r, g, b] = hexToRgb(hex);
  return 0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b);
}

/** Readable text colour (dark navy or white) for a given background. */
export const onColor = (bg: string): string => (luminance(bg) > 0.35 ? '#06231d' : '#ffffff');

/** Brand gradient partner: hue-shift towards blue (teal -> blue is about +48deg). */
export function partner(hex: string): string {
  const [h, s, l] = hexToHsl(hex);
  return hslToHex((h + 48) % 360, Math.max(s, 0.6), clampL(l));
}
const clampL = (l: number) => Math.min(0.62, Math.max(0.45, l));

export function mix(a: string, b: string, t: number): string {
  const [r1, g1, b1] = hexToRgb(a), [r2, g2, b2] = hexToRgb(b);
  return rgbToHex(r1 + (r2 - r1) * t, g1 + (g2 - g1) * t, b1 + (b2 - b1) * t);
}
