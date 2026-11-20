import type { ClassStyle, MoveClass, Settings, TacticTag, TagStyle } from './types';

const c = (label: string, color: string, icon: string): ClassStyle => ({ enabled: true, label, color, icon });
const t = (label: string, color: string, icon: string, enabled = true): TagStyle => ({ enabled, label, color, icon });

export const DEFAULT_CLASSES: Record<MoveClass, ClassStyle> = {
  brilliant: c('Brilliant', '#1bbfb0', '!!'),
  great: c('Great', '#5b8bb9', '!'),
  best: c('Best', '#6fbf4a', '★'),
  excellent: c('Excellent', '#8cc152', '👍'),
  good: c('Good', '#9bb26b', '✓'),
  inaccuracy: c('Inaccuracy', '#f7c631', '?!'),
  mistake: c('Mistake', '#e58f2a', '?'),
  blunder: c('Blunder', '#ca3431', '??'),
};

export const DEFAULT_TAGS: Record<TacticTag, TagStyle> = {
  fork: t('Fork', '#b15cff', '⑂'),
  pin: t('Pin', '#ff7ab6', '📌'),
  skewer: t('Skewer', '#ff9f43', '⟿'),
  discovered: t('Discovered', '#4cc9f0', '◐'),
  hanging: t('Hanging', '#ff5d73', '⚓'),
  check: t('Check', '#ffd166', '+'),
  mate: t('Mate', '#ef476f', '#'),
  capture: t('Capture', '#8d99ae', '×', false),
  promotion: t('Promotion', '#06d6a0', '♛'),
  sacrifice: t('Sacrifice', '#f78c6b', '⚔'),
  castle: t('Castle', '#8d99ae', '♜', false),
  enPassant: t('En passant', '#8d99ae', 'e.p.', false),
};

export const DEFAULT_SETTINGS: Settings = {
  version: 1,
  acceptedFairPlay: false,
  site: 'chessCom',
  arrows: {
    show: true, count: 3, style: 'classic', curveKnightMoves: true,
    opacity: 0.85, widthScale: 1, glow: true, animate: true,
    colorMode: 'rank',
    rankColors: ['#22d3b0', '#3b82ff', '#a78bfa', '#f59e0b', '#94a3b8'],
    forSide: 'sideToMove', minClass: null,
  },
  labels: {
    show: true, placement: 'cornerBadge', style: 'iconText', size: 1,
    showTags: true, maxTags: 2,
    classes: DEFAULT_CLASSES, tags: DEFAULT_TAGS,
  },
  evalBar: {
    show: true, position: 'left', width: 14, showNumber: true, style: 'gradient', animate: true,
    whiteColor: '#f4f8fc', blackColor: '#101a26',
  },
  hud: {
    show: true, corner: 'br', lines: 3, showPv: true, showExplanation: true,
    showDepth: true, showWinPct: true, opacity: 0.94, scale: 1, compact: false,
  },
  engine: { threads: 2, hashMb: 128, multiPv: 3, depthLimit: null, moveTimeMs: null, skillLevel: null, auto: true, minDisplayDepth: 12 },
  capture: { fps: 8, targetId: null },
  theme: { mode: 'dark', accent: '#22d3b0', uiScale: 1, reduceMotion: false },
  hotkeys: {
    toggleOverlay: 'Alt+Space', toggleHud: 'Alt+H', cycleArrows: 'Alt+A', recalibrate: 'Alt+R',
  },
};
