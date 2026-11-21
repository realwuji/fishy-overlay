// Validation / clamping of Settings to the ranges documented in shared/types.ts,
// plus deep-merge, hotkey and FEN validators.
import { DEFAULT_SETTINGS } from '../shared/defaults';
import {
  MOVE_CLASSES, TACTIC_TAGS,
  type ArrowStyle, type ClassStyle, type Corner, type MoveClass, type Settings, type Site, type TacticTag, type TagStyle,
} from '../shared/types';
import { clone, clamp } from './dom';
import { normHex } from './color';

type Rec = Record<string, unknown>;
const rec = (v: unknown): Rec => (v && typeof v === 'object' && !Array.isArray(v) ? (v as Rec) : {});

const bool = (v: unknown, d: boolean): boolean => (typeof v === 'boolean' ? v : d);
const num = (v: unknown, lo: number, hi: number, d: number, int = false): number => {
  if (typeof v !== 'number' || !Number.isFinite(v)) return d;
  const x = clamp(v, lo, hi);
  return int ? Math.round(x) : Math.round(x * 1000) / 1000;
};
const numOrNull = (v: unknown, lo: number, hi: number, d: number | null, int = true): number | null =>
  v === null ? null : v === undefined ? d : num(v, lo, hi, d ?? lo, int);
const oneOf = <T>(v: unknown, list: readonly T[], d: T): T => (list.includes(v as T) ? (v as T) : d);
const color = (v: unknown, d: string): string => (typeof v === 'string' ? normHex(v) ?? d : d);
const text = (v: unknown, d: string, max: number): string =>
  typeof v === 'string' ? Array.from(v).slice(0, max).join('') : d;

export const ARROW_STYLES: ArrowStyle[] = ['classic', 'slim', 'dotted', 'pill'];
export const CORNERS: Corner[] = ['tl', 'tr', 'bl', 'br'];
export const SITES: Site[] = ['chessCom', 'lichess', 'generic'];

export function recommendedThreads(): number {
  const hw = navigator.hardwareConcurrency || 4;
  return clamp(Math.floor(hw / 2), 1, 8);
}
export const maxThreads = (): number => clamp(navigator.hardwareConcurrency || 4, 1, 128);

// ---------------------------------------------------------------- hotkeys
const MODS = ['Ctrl', 'Alt', 'Shift', 'Cmd'] as const;
const KEY_RE = /^([A-Z0-9]|F([1-9]|1[0-9]|2[0-4])|Space|Enter|Tab|Up|Down|Left|Right|Home|End|PageUp|PageDown|Insert|Delete|Minus|Equal|Comma|Period|Slash|Semicolon|Quote|Backquote|BracketLeft|BracketRight|Backslash)$/;

/** Returns an error message, or null when the accelerator is valid (e.g. "Alt+Space"). */
export function hotkeyError(s: string): string | null {
  const parts = s.split('+').map((p) => p.trim());
  const key = parts.pop() ?? '';
  if (!KEY_RE.test(key)) return 'Unsupported key. Use a letter, number, F-key, Space, arrow or similar.';
  const mods = parts as string[];
  if (new Set(mods).size !== mods.length) return 'Repeated modifier.';
  if (mods.some((m) => !(MODS as readonly string[]).includes(m))) return 'Modifiers must be Ctrl, Alt, Shift or Cmd.';
  const isF = /^F\d+$/.test(key);
  if (!isF && !mods.some((m) => m !== 'Shift')) return 'Include Ctrl, Alt or Cmd so it does not clash with typing.';
  return null;
}

export function normalizeHotkey(s: string): string {
  const parts = s.split('+').map((p) => p.trim());
  const key = parts.pop() ?? '';
  const mods = MODS.filter((m) => parts.includes(m));
  return [...mods, key].join('+');
}

// -------------------------------------------------------------------- FEN
export function checkFen(input: string): { ok: true; fen: string } | { ok: false; error: string } {
  const f = input.trim().split(/\s+/);
  if (f.length < 4 || f.length > 6) return { ok: false, error: 'A FEN has 4 to 6 space-separated fields.' };
  const ranks = f[0].split('/');
  if (ranks.length !== 8) return { ok: false, error: 'The board needs 8 ranks separated by "/".' };
  let wk = 0, bk = 0;
  for (let i = 0; i < 8; i++) {
    let n = 0;
    for (const ch of ranks[i]) {
      if (/[1-8]/.test(ch)) n += Number(ch);
      else if (/[pnbrqkPNBRQK]/.test(ch)) {
        n += 1;
        if (ch === 'K') wk++;
        if (ch === 'k') bk++;
        if ((i === 0 || i === 7) && /[pP]/.test(ch)) return { ok: false, error: 'Pawns cannot stand on the first or last rank.' };
      } else return { ok: false, error: `Unknown character "${ch}" on rank ${8 - i}.` };
    }
    if (n !== 8) return { ok: false, error: `Rank ${8 - i} does not add up to 8 squares.` };
  }
  if (wk !== 1 || bk !== 1) return { ok: false, error: 'Each side needs exactly one king.' };
  if (!/^[wb]$/.test(f[1])) return { ok: false, error: 'Side to move must be "w" or "b".' };
  if (!/^(-|[KQkq]{1,4})$/.test(f[2])) return { ok: false, error: 'Castling rights must be "-" or letters from KQkq.' };
  if (!/^(-|[a-h][36])$/.test(f[3])) return { ok: false, error: 'En passant square must be "-" or like "e3".' };
  const half = f[4] ?? '0', full = f[5] ?? '1';
  if (!/^\d+$/.test(half) || !/^\d+$/.test(full)) return { ok: false, error: 'Move counters must be whole numbers.' };
  return { ok: true, fen: [f[0], f[1], f[2], f[3], half, full].join(' ') };
}

// ---------------------------------------------------------------- settings
function sanClass(v: unknown, d: ClassStyle): ClassStyle {
  const r = rec(v);
  return { enabled: bool(r.enabled, d.enabled), label: text(r.label, d.label, 24), color: color(r.color, d.color), icon: text(r.icon, d.icon, 4) };
}
function sanTag(v: unknown, d: TagStyle): TagStyle {
  const r = rec(v);
  return { enabled: bool(r.enabled, d.enabled), label: text(r.label, d.label, 24), color: color(r.color, d.color), icon: text(r.icon, d.icon, 4) };
}

/** Coerce anything (stored JSON, Rust payload, imported file) into a valid Settings. */
export function sanitize(raw: unknown): Settings {
  const D = DEFAULT_SETTINGS;
  const r = rec(raw);
  const a = rec(r.arrows), l = rec(r.labels), e = rec(r.evalBar), hd = rec(r.hud), en = rec(r.engine);
  const cp = rec(r.capture), th = rec(r.theme), hk = rec(r.hotkeys), pk = rec(r.peek);

  const rankIn = Array.isArray(a.rankColors) ? a.rankColors : [];
  const rankColors = D.arrows.rankColors.map((d, i) => color(rankIn[i], d));

  const classes = {} as Record<MoveClass, ClassStyle>;
  const cin = rec(l.classes);
  for (const k of MOVE_CLASSES) classes[k] = sanClass(cin[k], D.labels.classes[k]);
  const tags = {} as Record<TacticTag, TagStyle>;
  const tin = rec(l.tags);
  for (const k of TACTIC_TAGS) tags[k] = sanTag(tin[k], D.labels.tags[k]);

  const hotkeys = { ...D.hotkeys } as Settings['hotkeys'];
  for (const k of Object.keys(D.hotkeys) as (keyof Settings['hotkeys'])[]) {
    const v = hk[k];
    hotkeys[k] = typeof v === 'string' && !hotkeyError(v) ? normalizeHotkey(v) : D.hotkeys[k];
  }

  return {
    version: 1,
    acceptedFairPlay: bool(r.acceptedFairPlay, D.acceptedFairPlay),
    site: oneOf(r.site, SITES, D.site),
    arrows: {
      show: bool(a.show, D.arrows.show),
      count: num(a.count, 1, 5, D.arrows.count, true),
      style: oneOf(a.style, ARROW_STYLES, D.arrows.style),
      curveKnightMoves: bool(a.curveKnightMoves, D.arrows.curveKnightMoves),
      opacity: num(a.opacity, 0.1, 1, D.arrows.opacity),
      widthScale: num(a.widthScale, 0.5, 1.6, D.arrows.widthScale),
      glow: bool(a.glow, D.arrows.glow),
      animate: bool(a.animate, D.arrows.animate),
      colorMode: oneOf(a.colorMode, ['rank', 'classification'] as const, D.arrows.colorMode),
      rankColors,
      forSide: oneOf(a.forSide, ['sideToMove', 'white', 'black', 'bottom'] as const, D.arrows.forSide),
      minClass: a.minClass === null ? null : oneOf<MoveClass | null>(a.minClass, MOVE_CLASSES, D.arrows.minClass),
    },
    labels: {
      show: bool(l.show, D.labels.show),
      placement: oneOf(l.placement, ['cornerBadge', 'arrowTail', 'arrowHead'] as const, D.labels.placement),
      style: oneOf(l.style, ['icon', 'text', 'iconText'] as const, D.labels.style),
      size: num(l.size, 0.6, 1.6, D.labels.size),
      showTags: bool(l.showTags, D.labels.showTags),
      maxTags: num(l.maxTags, 0, 3, D.labels.maxTags, true),
      classes, tags,
    },
    evalBar: {
      show: bool(e.show, D.evalBar.show),
      position: oneOf(e.position, ['left', 'right'] as const, D.evalBar.position),
      width: num(e.width, 6, 40, D.evalBar.width, true),
      showNumber: bool(e.showNumber, D.evalBar.showNumber),
      style: oneOf(e.style, ['solid', 'gradient'] as const, D.evalBar.style),
      animate: bool(e.animate, D.evalBar.animate),
      whiteColor: color(e.whiteColor, D.evalBar.whiteColor),
      blackColor: color(e.blackColor, D.evalBar.blackColor),
    },
    hud: {
      show: bool(hd.show, D.hud.show),
      corner: oneOf(hd.corner, CORNERS, D.hud.corner),
      lines: num(hd.lines, 1, 5, D.hud.lines, true),
      showPv: bool(hd.showPv, D.hud.showPv),
      showExplanation: bool(hd.showExplanation, D.hud.showExplanation),
      showDepth: bool(hd.showDepth, D.hud.showDepth),
      showWinPct: bool(hd.showWinPct, D.hud.showWinPct),
      opacity: num(hd.opacity, 0.2, 1, D.hud.opacity),
      scale: num(hd.scale, 0.6, 1.6, D.hud.scale),
      compact: bool(hd.compact, D.hud.compact),
    },
    engine: {
      threads: num(en.threads, 1, 128, D.engine.threads, true),
      hashMb: num(en.hashMb, 16, 8192, D.engine.hashMb, true),
      multiPv: num(en.multiPv, 1, 8, D.engine.multiPv, true),
      depthLimit: numOrNull(en.depthLimit, 1, 60, D.engine.depthLimit),
      moveTimeMs: numOrNull(en.moveTimeMs, 50, 120000, D.engine.moveTimeMs),
      skillLevel: numOrNull(en.skillLevel, 0, 20, D.engine.skillLevel),
      auto: bool(en.auto, D.engine.auto ?? true),
      minDisplayDepth: num(en.minDisplayDepth, 0, 30, D.engine.minDisplayDepth ?? 12, true),
    },
    capture: {
      fps: num(cp.fps, 1, 30, D.capture.fps, true),
      targetId: typeof cp.targetId === 'string' ? cp.targetId : null,
    },
    theme: {
      mode: oneOf(th.mode, ['dark', 'light', 'system'] as const, D.theme.mode),
      accent: color(th.accent, D.theme.accent),
      uiScale: num(th.uiScale, 0.8, 1.4, D.theme.uiScale),
      reduceMotion: bool(th.reduceMotion, D.theme.reduceMotion),
    },
    hotkeys,
    peek: {
      mode: oneOf(pk.mode, ['off', 'hold', 'toggle'] as const, D.peek.mode),
      fadeMs: num(pk.fadeMs, 0, 1000, D.peek.fadeMs, true),
      autoHideSec: num(pk.autoHideSec, 0, 120, D.peek.autoHideSec, true),
      indicator: bool(pk.indicator, D.peek.indicator),
    },
  };
}

export type DeepPartial<T> = { [K in keyof T]?: T[K] extends object ? (T[K] extends unknown[] ? T[K] : DeepPartial<T[K]>) : T[K] };

/** Deep merge of plain objects; arrays and primitives in `patch` replace. Returns a new object. */
export function deepMerge<T>(base: T, patch: unknown): T {
  if (patch === undefined) return clone(base);
  if (!patch || typeof patch !== 'object' || Array.isArray(patch) || !base || typeof base !== 'object' || Array.isArray(base)) {
    return clone(patch as T);
  }
  const out: Rec = clone(base) as Rec;
  for (const [k, v] of Object.entries(patch as Rec)) out[k] = deepMerge(out[k], v);
  return out as T;
}

export const jsonEqual = (a: unknown, b: unknown): boolean => JSON.stringify(a) === JSON.stringify(b);
