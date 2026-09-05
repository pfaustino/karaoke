export const NOTE_NAMES = ['C', 'C#', 'D', 'D#', 'E', 'F', 'F#', 'G', 'G#', 'A', 'A#', 'B'] as const;

export const SCALE_IDS = ['chromatic', 'major', 'minor'] as const;
export type ScaleId = (typeof SCALE_IDS)[number];

/** Pitch classes relative to the selected key tonic. */
export const SCALE_STEPS: Record<ScaleId, readonly number[]> = {
  chromatic: [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11],
  major: [0, 2, 4, 5, 7, 9, 11],
  minor: [0, 2, 3, 5, 7, 8, 10],
};

const A4_HZ = 440;
const A4_MIDI = 69;
const SEARCH_RADIUS = 12;
const MIN_FREQ = 70;
const MAX_FREQ = 1200;
const MIN_RATIO = 0.5;
const MAX_RATIO = 2;

export function freqToMidi(freq: number): number {
  return A4_MIDI + 12 * Math.log2(freq / A4_HZ);
}

export function midiToFreq(midi: number): number {
  return A4_HZ * 2 ** ((midi - A4_MIDI) / 12);
}

export function midiToNoteName(midi: number): string {
  const rounded = Math.round(midi);
  const pc = ((rounded % 12) + 12) % 12;
  const octave = Math.floor(rounded / 12) - 1;
  return `${NOTE_NAMES[pc]}${octave}`;
}

export function isScaleId(value: string): value is ScaleId {
  return (SCALE_IDS as readonly string[]).includes(value);
}

export function snapMidi(midi: number, key: number, scale: ScaleId): number {
  const tonic = ((Math.round(key) % 12) + 12) % 12;
  const steps = SCALE_STEPS[scale];
  const lo = Math.floor(midi) - SEARCH_RADIUS;
  const hi = Math.floor(midi) + SEARCH_RADIUS;
  let bestMidi = Math.round(midi);
  let bestDist = Number.POSITIVE_INFINITY;

  for (let candidate = lo; candidate <= hi; candidate += 1) {
    const pitchClass = ((candidate % 12) + 12) % 12;
    const degree = (pitchClass - tonic + 12) % 12;
    if (!steps.includes(degree)) continue;
    const dist = Math.abs(candidate - midi);
    if (dist < bestDist) {
      bestDist = dist;
      bestMidi = candidate;
    }
  }

  return bestMidi;
}

export function correctionRatio(freq: number, key: number, scale: ScaleId): number {
  if (!Number.isFinite(freq) || freq < MIN_FREQ || freq > MAX_FREQ) {
    return 1;
  }
  const midi = freqToMidi(freq);
  const target = snapMidi(midi, key, scale);
  const ratio = midiToFreq(target) / freq;
  return Math.min(MAX_RATIO, Math.max(MIN_RATIO, ratio));
}

export function centsOff(freq: number, targetMidi: number): number {
  if (!Number.isFinite(freq) || freq <= 0) return 0;
  return 100 * (freqToMidi(freq) - targetMidi);
}
