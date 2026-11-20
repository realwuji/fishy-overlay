export const SECTION_IDS = ['overview', 'arrows', 'labels', 'evalhud', 'engine', 'capture', 'appearance', 'presets', 'about'] as const;
export type SectionId = (typeof SECTION_IDS)[number];

export const navigate = (id: SectionId): void => { location.hash = id; };
