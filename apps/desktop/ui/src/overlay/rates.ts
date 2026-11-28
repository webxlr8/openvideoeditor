// Playback rates (add-viewer-interaction-and-text-fills, design D5): the ones the core's transport accepts.

export const PLAYBACK_RATES: number[] = [0.5, 1, 1.25, 1.5, 2];

export function rateLabel(rate: number): string {
  return `${rate}×`;
}
