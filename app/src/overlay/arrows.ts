// Candidate-move arrows plus their class badges and tactic-tag chips.
// Each arrow is a persistent entity keyed by UCI so that, between frames, arrows
// slide to new squares, fade in/out, and re-colour instead of snapping.
import type { AnalysisFrame, BoardGeometry, CandidateMove, MoveClass, Settings, TacticTag } from '../shared/types';
import { Spring, clamp, lerp } from './anim';
import { classIndex, displayCol, displayRow, isKnightMove, parseSquare } from './chess';
import { blueOf, greenOf, onColor, packRgb, parseColor, redOf, rgba } from './color';
import { Env, font, roundRect, snap, textWidth } from './env';

interface BadgeModel {
  kind: 'icon' | 'text' | 'iconText';
  color: number;
  fg: number;
  icon: string;
  text: string;
  iconW10: number;
  textW10: number;
}

interface ChipModel {
  color: number;
  icon: string;
  text: string;
  iconW10: number;
  textW10: number;
}

interface Ent {
  key: string;
  cand: CandidateMove;
  live: boolean;
  knight: boolean;
  fx: Spring; fy: Spring; tx: Spring; ty: Spring;   // board units (square centres), display orientation
  a: Spring;      // presence
  pop: Spring;    // badge pop-in scale (slightly under-damped)
  rk: Spring;     // animated rank index (0 = best)
  cr: Spring; cg: Spring; cb: Spring;
  badge: BadgeModel | null;
  chips: ChipModel[];
  // Scratch written by the arrow pass, read by the label pass (CSS px).
  tailX: number; tailY: number; tailDx: number; tailDy: number;
  tipX: number; tipY: number; tipDx: number; tipDy: number;
  cX: number; cY: number; elbow: boolean;
}

export interface BoardRect { x: number; y: number; s: number; flipped: boolean }

const DASH = [0.01, 10];
const NO_DASH: number[] = [];
const BADGE_FONT_W = 700;

const sq = { file: 0, rank: 0 };

function shaftPath(
  ctx: CanvasRenderingContext2D, sx: number, sy: number, elbow: boolean, cx: number, cy: number,
  bx: number, by: number, rad: number,
): void {
  ctx.beginPath();
  ctx.moveTo(sx, sy);
  if (elbow) ctx.arcTo(cx, cy, bx, by, rad);
  ctx.lineTo(bx, by);
}

/** Rounded triangle: tip (x2, y2) and the two base corners around (bx, by). */
function headPath(
  ctx: CanvasRenderingContext2D, x2: number, y2: number, bx: number, by: number,
  px: number, py: number, hw: number, round: number,
): void {
  const ax = bx + px * hw, ay = by + py * hw;
  const cx = bx - px * hw, cy = by - py * hw;
  ctx.beginPath();
  ctx.moveTo((x2 + ax) / 2, (y2 + ay) / 2);
  ctx.arcTo(ax, ay, cx, cy, round);
  ctx.arcTo(cx, cy, x2, y2, round);
  ctx.arcTo(x2, y2, ax, ay, round * 1.4);
  ctx.closePath();
}


/** Longest label drawn on the board; longer text is cut with an ellipsis so a pill never spans several squares. */
const MAX_LABEL_CHARS = 14;
function clipLabel(s: string): string {
  const chars = Array.from(s);
  return chars.length > MAX_LABEL_CHARS ? `${chars.slice(0, MAX_LABEL_CHARS - 1).join('').trimEnd()}\u2026` : s;
}

export class ArrowLayer {
  private ents: Ent[] = [];
  private order: Ent[] = [];
  private st: Settings | null = null;
  private labelVis = new Spring(1, 16, 1, 0.004);
  private fen = '';
  private lastFlipped = false;
  private placed = new Float32Array(96);
  private placedN = 0;

  /** Rebuild the desired arrow set from the latest inputs and retarget all springs. */
  sync(env: Env, frame: AnalysisFrame | null, geometry: BoardGeometry | null, st: Settings, animateBase: boolean): void {
    this.st = st;
    const A = st.arrows;
    const animate = animateBase && A.animate;
    if (geometry) this.lastFlipped = geometry.flipped;
    const flipped = this.lastFlipped;
    this.labelVis.set(st.labels.show ? 1 : 0, animateBase);

    const fenChanged = frame !== null && frame.fen !== this.fen;
    if (frame) this.fen = frame.fen;

    for (let i = 0; i < this.ents.length; i++) this.ents[i].live = false;

    if (A.show && frame) {
      const count = clamp(Math.round(A.count), 1, 5);
      const minIdx = A.minClass ? classIndex(A.minClass) : 99;
      let side: 'w' | 'b' | null = null;
      if (A.forSide === 'white') side = 'w';
      else if (A.forSide === 'black') side = 'b';
      else if (A.forSide === 'bottom') side = flipped ? 'b' : 'w';
      if (side === null || frame.sideToMove === side) {
        let n = 0;
        for (let i = 0; i < frame.candidates.length && n < count; i++) {
          const c = frame.candidates[i];
          if (classIndex(c.cls) > minIdx) continue;
          if (!parseSquare(c.from, sq)) continue;
          const ff = sq.file, fr = sq.rank;
          if (!parseSquare(c.to, sq)) continue;
          if (ff === sq.file && fr === sq.rank) continue;
          this.upsert(env, c, n, ff, fr, sq.file, sq.rank, flipped, animate, fenChanged, st);
          n++;
        }
      }
    }

    for (let i = 0; i < this.ents.length; i++) {
      const e = this.ents[i];
      if (!e.live) { e.a.w = 30; e.a.set(0, animate); }
    }
    if (!animate) this.prune();
  }

  private upsert(
    env: Env, c: CandidateMove, rank: number, ff: number, fr: number, tf: number, tr: number,
    flipped: boolean, animate: boolean, fenChanged: boolean, st: Settings,
  ): void {
    let e: Ent | undefined;
    for (let i = 0; i < this.ents.length; i++) {
      if (this.ents[i].key === c.uci) { e = this.ents[i]; break; }
    }
    const fxT = displayCol(ff, flipped) + 0.5;
    const fyT = displayRow(fr, flipped) + 0.5;
    const txT = displayCol(tf, flipped) + 0.5;
    const tyT = displayRow(tr, flipped) + 0.5;
    const col = this.colorFor(c, rank, st);
    if (!e) {
      e = {
        key: c.uci, cand: c, live: true, knight: false,
        fx: new Spring(fxT, 24, 1, 0.002), fy: new Spring(fyT, 24, 1, 0.002),
        // New arrows grow out of their tail square.
        tx: new Spring(animate ? fxT : txT, 20, 1, 0.002), ty: new Spring(animate ? fyT : tyT, 20, 1, 0.002),
        a: new Spring(animate ? 0 : 1, 16, 1, 0.004),
        pop: new Spring(animate ? 0.2 : 1, 26, 0.5, 0.004),
        rk: new Spring(rank, 18, 1, 0.004),
        cr: new Spring(redOf(col), 20, 1, 0.5), cg: new Spring(greenOf(col), 20, 1, 0.5), cb: new Spring(blueOf(col), 20, 1, 0.5),
        badge: null, chips: [],
        tailX: 0, tailY: 0, tailDx: 1, tailDy: 0, tipX: 0, tipY: 0, tipDx: 1, tipDy: 0, cX: 0, cY: 0, elbow: false,
      };
      e.pop.t = 1;
      this.ents.push(e);
    } else {
      if (e.cand.cls !== c.cls && animate) { e.pop.x = 0.55; e.pop.v = 0; }
      e.cand = c;
    }
    e.live = true;
    e.a.w = 16;
    e.knight = isKnightMove(ff, fr, tf, tr);
    if (fenChanged && animate && e.a.x > 0.5) { e.pop.x = 0.5; e.pop.v = 0; }
    e.fx.set(fxT, animate); e.fy.set(fyT, animate);
    e.tx.set(txT, animate); e.ty.set(tyT, animate);
    e.a.set(1, animate);
    e.pop.set(1, animate);
    e.rk.set(rank, animate);
    e.cr.set(redOf(col), animate); e.cg.set(greenOf(col), animate); e.cb.set(blueOf(col), animate);
    this.buildLabels(env, e, st);
  }

  private colorFor(c: CandidateMove, rank: number, st: Settings): number {
    const A = st.arrows;
    if (A.colorMode === 'classification') {
      const cs = st.labels.classes[c.cls];
      if (cs) return parseColor(cs.color);
    }
    const rc = A.rankColors;
    return parseColor(rc[Math.min(rank, rc.length - 1)] ?? '#94a3b8');
  }

  // ------------------------------------------------------------- labels
  private buildLabels(env: Env, e: Ent, st: Settings): void {
    const { ctx } = env;
    const L = st.labels;
    e.badge = null;
    e.chips.length = 0;
    if (!L.show) return;
    const cs = L.classes[e.cand.cls as MoveClass];
    if (cs && cs.enabled) {
      const color = parseColor(cs.color);
      let kind = L.style;
      let icon = cs.icon;
      if (!icon && kind !== 'text') kind = 'text';
      if (!cs.label && kind !== 'icon') kind = 'icon';
      if (!icon && !cs.label) icon = '?';
      const f = font(BADGE_FONT_W, 10);
      const label = clipLabel(cs.label);
      e.badge = {
        kind, color, fg: onColor(color), icon, text: label,
        iconW10: icon ? textWidth(ctx, f, icon) : 0,
        textW10: label ? textWidth(ctx, f, label) : 0,
      };
    }
    if (L.showTags && L.maxTags > 0) {
      const f = font(BADGE_FONT_W, 10);
      const tags = e.cand.tags;
      for (let i = 0; i < tags.length && e.chips.length < Math.min(3, L.maxTags); i++) {
        const ts = L.tags[tags[i] as TacticTag];
        if (!ts || !ts.enabled) continue;
        const label = clipLabel(ts.label);
        e.chips.push({
          color: parseColor(ts.color), icon: ts.icon, text: label,
          iconW10: ts.icon ? textWidth(ctx, f, ts.icon) : 0,
          textW10: label ? textWidth(ctx, f, label) : 0,
        });
      }
    }
  }

  // --------------------------------------------------------------- tick
  step(dt: number): boolean {
    let active = this.labelVis.step(dt);
    for (let i = 0; i < this.ents.length; i++) {
      const e = this.ents[i];
      // Non-short-circuit: every spring must advance.
      const m =
        Number(e.fx.step(dt)) | Number(e.fy.step(dt)) | Number(e.tx.step(dt)) | Number(e.ty.step(dt)) |
        Number(e.a.step(dt)) | Number(e.pop.step(dt)) | Number(e.rk.step(dt)) |
        Number(e.cr.step(dt)) | Number(e.cg.step(dt)) | Number(e.cb.step(dt));
      if (m) active = true;
    }
    this.prune();
    return active;
  }

  private prune(): void {
    for (let i = this.ents.length - 1; i >= 0; i--) {
      const e = this.ents[i];
      if (!e.live && e.a.t === 0 && e.a.x === 0) {
        this.ents[i] = this.ents[this.ents.length - 1];
        this.ents.pop();
      }
    }
  }

  private sortBy(desc: boolean): Ent[] {
    const o = this.order;
    o.length = 0;
    for (let i = 0; i < this.ents.length; i++) o.push(this.ents[i]);
    for (let i = 1; i < o.length; i++) {
      const v = o[i];
      let j = i - 1;
      while (j >= 0 && (desc ? o[j].rk.x < v.rk.x : o[j].rk.x > v.rk.x)) { o[j + 1] = o[j]; j--; }
      o[j + 1] = v;
    }
    return o;
  }

  // --------------------------------------------------------------- draw
  draw(
    env: Env, g: BoardRect, alpha: number,
    avoid: { x: number; y: number; w: number; h: number } | null = null,
    avoidBar: { x: number; y: number; w: number; h: number } | null = null,
  ): void {
    const st = this.st;
    if (!st || this.ents.length === 0) return;
    const s = g.s / 8;
    const order = this.sortBy(true);
    for (let i = 0; i < order.length; i++) this.drawArrow(env, order[i], g.x, g.y, s, alpha);
    if (st.labels.show || this.labelVis.x > 0.01) this.drawLabels(env, g, s, alpha, avoid, avoidBar);
  }

  private drawArrow(env: Env, e: Ent, gx: number, gy: number, s: number, alpha: number): void {
    const A = (this.st as Settings).arrows;
    const { ctx, dpr } = env;
    const rk = clamp(e.rk.x, 0, 4) / 4;
    const wMul = lerp(1.14, 0.86, rk);
    const col = packRgb(e.cr.x, e.cg.x, e.cb.x);
    const a = clamp(alpha * e.a.x * A.opacity * lerp(1, 0.74, rk), 0, 1);
    const x0 = gx + e.fx.x * s;
    const y0 = gy + e.fy.x * s;
    const x2 = gx + e.tx.x * s;
    const y2 = gy + e.ty.x * s;
    const dx = x2 - x0;
    const dy = y2 - y0;
    const len = Math.hypot(dx, dy);
    if (len < 1.5) return;

    const ws = A.widthScale * wMul;
    const hs = 0.55 + 0.45 * ws;
    const style = A.style;
    let shaft: number, headW: number, headL: number;
    switch (style) {
      case 'slim': shaft = 0.07 * s * ws; headW = 0.3 * s * hs; headL = 0.32 * s * hs; break;
      case 'dotted': shaft = 0.105 * s * ws; headW = 0.44 * s * hs; headL = 0.38 * s * hs; break;
      case 'pill': shaft = 0.3 * s * ws; headW = 0; headL = shaft * 0.5; break;
      default: shaft = 0.185 * s * ws; headW = 0.52 * s * hs; headL = 0.44 * s * hs; break;
    }

    // Path: tail -> (corner) -> base of head -> tip.
    let elbow = e.knight && A.curveKnightMoves;
    let cx = x0, cy = y0;
    if (elbow) {
      if (Math.abs(dx) >= Math.abs(dy)) { cx = x2; cy = y0; } else { cx = x0; cy = y2; }
      if (Math.hypot(cx - x0, cy - y0) < 2 || Math.hypot(x2 - cx, y2 - cy) < 2) elbow = false;
    }
    const lx = elbow ? cx : x0;
    const ly = elbow ? cy : y0;
    const ll = Math.hypot(x2 - lx, y2 - ly);
    const ux = (x2 - lx) / ll;
    const uy = (y2 - ly) / ll;
    headL = Math.min(headL, ll * 0.82);
    const bxp = x2 - ux * headL;
    const byp = y2 - uy * headL;
    const fl = elbow ? Math.hypot(cx - x0, cy - y0) : len;
    const fdx = elbow ? (cx - x0) / fl : dx / len;
    const fdy = elbow ? (cy - y0) / fl : dy / len;
    let sx = x0, sy = y0;
    if (style === 'pill') { sx += fdx * shaft * 0.5; sy += fdy * shaft * 0.5; }
    e.tailX = x0; e.tailY = y0; e.tailDx = fdx; e.tailDy = fdy;
    e.tipX = x2; e.tipY = y2; e.tipDx = ux; e.tipDy = uy;
    e.cX = cx; e.cY = cy; e.elbow = elbow;

    const hasShaft = Math.hypot(bxp - sx, byp - sy) > 0.75;
    const rad = elbow ? clamp(Math.min(0.34 * s, (ll - headL) * 0.9, fl * 0.5), 0.5, 1e4) : 0;
    const px = -uy;
    const py = ux;
    const hw = headW / 2;
    const round = Math.min(0.05 * s, hw * 0.25);

    ctx.save();
    ctx.lineJoin = 'round';
    ctx.lineCap = style === 'pill' || style === 'dotted' ? 'round' : 'butt';
    ctx.lineWidth = shaft;

    // 1) Dark under-stroke keeps arrows readable on light and dark squares.
    if (style !== 'dotted' && a > 0.02) {
      ctx.strokeStyle = rgba(0x05080d, 0.28 * a);
      ctx.fillStyle = ctx.strokeStyle;
      if (hasShaft) { ctx.lineWidth = shaft + 2.5; shaftPath(ctx, sx, sy, elbow, cx, cy, bxp, byp, rad); ctx.stroke(); }
      if (headW > 0) { ctx.lineWidth = 2.5; headPath(ctx, x2, y2, bxp, byp, px, py, hw, round); ctx.stroke(); }
    }

    // 2) Coloured body with optional glow.
    if (A.glow) {
      ctx.shadowColor = rgba(col, clamp(a * 0.95, 0, 1));
      ctx.shadowBlur = lerp(0.46, 0.24, rk) * s * dpr;
    }
    ctx.strokeStyle = rgba(col, a);
    ctx.fillStyle = rgba(col, a);
    ctx.lineWidth = shaft;
    if (hasShaft) {
      if (style === 'dotted') {
        DASH[1] = shaft * 1.9;
        ctx.setLineDash(DASH);
      }
      shaftPath(ctx, sx, sy, elbow, cx, cy, bxp, byp, rad);
      ctx.stroke();
      if (style === 'dotted') ctx.setLineDash(NO_DASH);
    }
    if (headW > 0) { headPath(ctx, x2, y2, bxp, byp, px, py, hw, round); ctx.fill(); }
    ctx.shadowColor = 'transparent';
    ctx.shadowBlur = 0;

    // 3) Pill style: a chevron near the tip reads as direction.
    if (style === 'pill') {
      const back = shaft * 0.62;
      const half = shaft * 0.26;
      ctx.beginPath();
      ctx.moveTo(x2 - ux * back + px * half, y2 - uy * back + py * half);
      ctx.lineTo(x2 - ux * (back - half * 1.1), y2 - uy * (back - half * 1.1));
      ctx.lineTo(x2 - ux * back - px * half, y2 - uy * back - py * half);
      ctx.lineWidth = Math.max(1.5, shaft * 0.16);
      ctx.lineCap = 'round';
      ctx.strokeStyle = rgba(0xffffff, 0.92 * a);
      ctx.stroke();
    }
    ctx.restore();
  }

  // -------------------------------------------------------------- labels
  private overlaps(x: number, y: number, w: number, h: number): boolean {
    const p = this.placed;
    for (let i = 0; i < this.placedN; i++) {
      const o = i * 4;
      if (x < p[o] + p[o + 2] && x + w > p[o] && y < p[o + 1] + p[o + 3] && y + h > p[o + 1]) return true;
    }
    return false;
  }

  /** Number of sample points along arrow bodies that fall inside the rectangle. */
  private arrowHits(x: number, y: number, w: number, h: number, skip: Ent | null, pad: number): number {
    let hits = 0;
    const ents = this.ents;
    for (let i = 0; i < ents.length; i++) {
      const e = ents[i];
      if (e === skip || e.a.x < 0.25) continue;
      hits += this.segHits(e.tailX, e.tailY, e.elbow ? e.cX : e.tipX, e.elbow ? e.cY : e.tipY, x, y, w, h, pad);
      if (e.elbow) hits += this.segHits(e.cX, e.cY, e.tipX, e.tipY, x, y, w, h, pad);
    }
    return hits;
  }

  private segHits(ax: number, ay: number, bx: number, by: number, x: number, y: number, w: number, h: number, pad: number): number {
    const len = Math.hypot(bx - ax, by - ay);
    const n = Math.max(1, Math.ceil(len / 7));
    let hits = 0;
    for (let k = 0; k <= n; k++) {
      const t = k / n;
      const px = ax + (bx - ax) * t;
      const py = ay + (by - ay) * t;
      if (px > x - pad && px < x + w + pad && py > y - pad && py < y + h + pad) hits++;
    }
    return hits;
  }

  private drawLabels(
    env: Env, g: BoardRect, s: number, alpha: number,
    avoid: { x: number; y: number; w: number; h: number } | null,
    avoidBar: { x: number; y: number; w: number; h: number } | null,
  ): void {
    const st = this.st as Settings;
    const L = st.labels;
    const { ctx, dpr, W, H } = env;
    const order = this.sortBy(false);
    this.placedN = 0;
    if (avoid) {
      // Keep labels clear of the HUD card.
      this.placed[0] = avoid.x - 6; this.placed[1] = avoid.y - 6; this.placed[2] = avoid.w + 12; this.placed[3] = avoid.h + 12;
      this.placedN = 1;
    }
    if (avoidBar) {
      // Keep labels off the eval bar and its number plate.
      const o = this.placedN * 4;
      this.placed[o] = avoidBar.x - 3; this.placed[o + 1] = avoidBar.y - 3;
      this.placed[o + 2] = avoidBar.w + 6; this.placed[o + 3] = avoidBar.h + 6;
      this.placedN++;
    }
    const size = L.size;
    const r = clamp(s * 0.19, 8.5, 20) * size;
    const cf = clamp(s * 0.16, 9.5, 15) * size;
    const chipH = cf * 1.85;
    const gapY = Math.max(2, cf * 0.28);
    const place = L.placement;

    for (let i = 0; i < order.length; i++) {
      const e = order[i];
      const a = alpha * e.a.x * this.labelVis.x * clamp(e.pop.x * 1.5, 0, 1);
      if (a < 0.02 || (!e.badge && e.chips.length === 0)) continue;
      const sc = clamp(e.pop.x, 0, 1.25);
      const b = e.badge;

      // Element sizes.
      let bw = 0;
      const bh = b ? 2 * r : 0;
      let iconFs = 0, textFs = 0;
      if (b) {
        iconFs = b.iconW10 > 0 ? Math.min(r * 1.12, (r * 1.45 * 10) / b.iconW10) : 0;
        textFs = r * 0.96;
        const tw = (b.textW10 * textFs) / 10;
        if (b.kind === 'icon') bw = bh;
        else if (b.kind === 'text') bw = tw + r * 1.3;
        else bw = bh + r * 0.1 + tw + r * 0.75;
      }
      const chipW = this.chipWidths(e, cf);
      let chipsW = 0;
      for (let k = 0; k < chipW.length; k++) chipsW = Math.max(chipsW, chipW[k]);
      const nChips = e.chips.length;
      const stackGap = b && nChips ? gapY : 0;
      const groupW = Math.max(bw, chipsW);
      const groupH = bh + stackGap + nChips * chipH + Math.max(0, nChips - 1) * gapY;

      // Square of the move's destination and arrow directions.
      const sqL = g.x + (e.tx.x - 0.5) * s;
      const sqT = g.y + (e.ty.x - 0.5) * s;
      let ax0: number, ay0: number, dxu: number, dyu: number;
      if (place === 'arrowTail') { ax0 = e.tailX + e.tailDx * s * 0.66; ay0 = e.tailY + e.tailDy * s * 0.66; dxu = e.tailDx; dyu = e.tailDy; }
      else { ax0 = e.tipX + e.tipDx * r * 0.95; ay0 = e.tipY + e.tipDy * r * 0.95; dxu = e.tipDx; dyu = e.tipDy; }
      const off = Math.max(groupW, groupH) * 0.5 + r * 0.6;

      // Try candidate placements; keep the one that hides the least.
      let bestScore = Infinity, bestX = 0, bestY = 0, bestAlign = 0; // align: 0 left, 1 centre, 2 right
      for (let k = 0; k < 6; k++) {
        let x: number, y: number, align: number;
        if (place === 'cornerBadge') {
          switch (k) {
            case 0: x = sqL + s + r * 0.3 - groupW; y = sqT - r * 0.35; align = 2; break;
            case 1: x = sqL + s + 4; y = sqT - r * 0.35; align = 0; break;
            case 2: x = sqL + s + r * 0.3 - groupW; y = sqT - groupH - 3; align = 2; break;
            case 3: x = sqL - groupW - 4; y = sqT - r * 0.35; align = 2; break;
            case 4: x = sqL + s + r * 0.3 - groupW; y = sqT + s + 3; align = 2; break;
            default: x = sqL + s + 4; y = sqT + s * 0.5; align = 0; break;
          }
        } else {
          const cxm = ax0 - dxu * 0, cym = ay0;
          let ox = 0, oy = 0;
          switch (k) {
            case 1: ox = -dyu * off; oy = dxu * off; break;
            case 2: ox = dyu * off; oy = -dxu * off; break;
            case 3: ox = dxu * off; oy = dyu * off; break;
            case 4: ox = -dxu * off; oy = -dyu * off; break;
            case 5: ox = -dyu * off * 1.6; oy = dxu * off * 1.6; break;
            default: break;
          }
          x = cxm + ox - groupW / 2; y = cym + oy - groupH / 2; align = 1;
        }
        const cx = clamp(x, 2, Math.max(2, W - groupW - 2));
        const cy = clamp(y, 2, Math.max(2, H - groupH - 2));
        let score = k * 2 + (Math.abs(cx - x) + Math.abs(cy - y)) * 0.6;
        if (this.overlaps(cx - 1, cy - 1, groupW + 2, groupH + 2)) score += 1000;
        // Chips must clear every arrow; the badge only has to clear the other arrows.
        if (nChips) {
          const chipTop = cy + bh + stackGap;
          score += 3 * this.arrowHits(cx - 2, chipTop - 2, chipsW + 4, groupH - bh - stackGap + 4, null, 5);
        }
        if (b) {
          const bxk = align === 2 ? cx + groupW - bw : align === 1 ? cx + (groupW - bw) / 2 : cx;
          score += 3 * this.arrowHits(bxk, cy, bw, bh, e, 4);
        }
        if (score < bestScore) { bestScore = score; bestX = cx; bestY = cy; bestAlign = align; }
        if (score < 1) break;
      }

      // Every spot hides another badge: leave this group out rather than draw over it.
      if (bestScore >= 1000) continue;

      if (this.placedN < 23) {
        const o = this.placedN * 4;
        this.placed[o] = bestX; this.placed[o + 1] = bestY; this.placed[o + 2] = groupW; this.placed[o + 3] = groupH;
        this.placedN++;
      }

      ctx.save();
      if (b) {
        const bxp = bestAlign === 2 ? bestX + groupW - bw : bestAlign === 1 ? bestX + (groupW - bw) / 2 : bestX;
        this.drawBadge(env, b, snap(bxp, dpr), snap(bestY, dpr), bw, bh, r, iconFs, textFs, a, sc);
      }
      let cyp = bestY + bh + stackGap;
      for (let k = 0; k < nChips; k++) {
        const w = chipW[k];
        const cxp = bestAlign === 2 ? bestX + groupW - w : bestAlign === 1 ? bestX + (groupW - w) / 2 : bestX;
        this.drawChip(env, e.chips[k], snap(cxp, dpr), snap(cyp, dpr), w, chipH, cf, a, sc);
        cyp += chipH + gapY;
      }
      ctx.restore();
    }
  }

  private chipScratch: number[] = [];
  private chipWidths(e: Ent, cf: number): number[] {
    const out = this.chipScratch;
    out.length = 0;
    for (let k = 0; k < e.chips.length; k++) {
      const c = e.chips[k];
      const iconBox = c.icon ? Math.max((c.iconW10 * cf) / 10, cf * 0.9) : 0;
      out.push(cf * 0.6 + iconBox + (c.icon ? cf * 0.4 : 0) + (c.textW10 * cf) / 10 + cf * 0.75);
    }
    return out;
  }

  private drawBadge(
    env: Env, b: BadgeModel, x: number, y: number, w: number, h: number, r: number,
    iconFs: number, textFs: number, a: number, sc: number,
  ): void {
    const { ctx, dpr } = env;
    const cxm = x + w / 2;
    const cym = y + h / 2;
    ctx.save();
    ctx.translate(cxm, cym);
    ctx.scale(sc, sc);
    ctx.globalAlpha = a;
    roundRect(ctx, -w / 2, -h / 2, w, h, h / 2);
    ctx.shadowColor = 'rgba(0,0,0,0.45)';
    ctx.shadowBlur = 7 * dpr;
    ctx.shadowOffsetY = 1.5 * dpr;
    ctx.fillStyle = rgba(b.color, 1);
    ctx.fill();
    ctx.shadowColor = 'transparent';
    ctx.shadowBlur = 0;
    ctx.shadowOffsetY = 0;
    ctx.lineWidth = Math.max(1.5, r * 0.13);
    ctx.strokeStyle = 'rgba(255,255,255,0.95)';
    ctx.stroke();

    ctx.textBaseline = 'middle';
    ctx.fillStyle = rgba(b.fg, 1);
    if (b.kind === 'icon') {
      ctx.textAlign = 'center';
      ctx.font = font(BADGE_FONT_W, iconFs);
      ctx.fillText(b.icon, 0, 0.5);
    } else if (b.kind === 'text') {
      ctx.textAlign = 'center';
      ctx.font = font(BADGE_FONT_W, textFs);
      ctx.fillText(b.text, 0, 0.5);
    } else {
      // Icon in a slightly darker disc, label to the right.
      const ix = -w / 2 + r;
      ctx.beginPath();
      ctx.arc(ix, 0, r - ctx.lineWidth, 0, Math.PI * 2);
      ctx.fillStyle = 'rgba(0,0,0,0.2)';
      ctx.fill();
      ctx.fillStyle = rgba(b.fg, 1);
      ctx.textAlign = 'center';
      ctx.font = font(BADGE_FONT_W, iconFs * 0.92);
      ctx.fillText(b.icon, ix, 0.5);
      ctx.textAlign = 'left';
      ctx.font = font(BADGE_FONT_W, textFs);
      ctx.fillText(b.text, -w / 2 + 2 * r + r * 0.1, 0.5);
    }
    ctx.restore();
  }

  private drawChip(
    env: Env, c: ChipModel, x: number, y: number, w: number, h: number, cf: number, a: number, sc: number,
  ): void {
    const { ctx } = env;
    ctx.save();
    ctx.translate(x + w / 2, y + h / 2);
    ctx.scale(sc, sc);
    ctx.globalAlpha = a;
    roundRect(ctx, -w / 2, -h / 2, w, h, h / 2);
    ctx.fillStyle = 'rgba(11,16,25,0.9)';
    ctx.shadowColor = 'rgba(0,0,0,0.4)';
    ctx.shadowBlur = 6 * env.dpr;
    ctx.shadowOffsetY = 1 * env.dpr;
    ctx.fill();
    ctx.shadowColor = 'transparent';
    ctx.shadowBlur = 0;
    ctx.shadowOffsetY = 0;
    ctx.lineWidth = 1.4;
    ctx.strokeStyle = rgba(c.color, 0.95);
    ctx.stroke();
    ctx.textBaseline = 'middle';
    ctx.textAlign = 'left';
    let tx = -w / 2 + cf * 0.6;
    if (c.icon) {
      const iconBox = Math.max((c.iconW10 * cf) / 10, cf * 0.9);
      ctx.font = font(BADGE_FONT_W, cf);
      ctx.fillStyle = rgba(c.color, 1);
      ctx.textAlign = 'center';
      ctx.fillText(c.icon, tx + iconBox / 2, 0.5);
      ctx.textAlign = 'left';
      tx += iconBox + cf * 0.4;
    }
    ctx.font = font(BADGE_FONT_W, cf);
    ctx.fillStyle = '#ffffff';
    ctx.fillText(c.text, tx, 0.5);
    ctx.restore();
  }
}
