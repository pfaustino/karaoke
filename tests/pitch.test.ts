import { describe, expect, it } from 'vitest';
import {
  centsOff,
  correctionRatio,
  freqToMidi,
  midiToFreq,
  midiToNoteName,
  snapMidi,
} from '../src/audio/pitch';

describe('pitch math', () => {
  it('round-trips A4', () => {
    expect(freqToMidi(440)).toBeCloseTo(69, 8);
    expect(midiToFreq(69)).toBeCloseTo(440, 8);
    expect(midiToNoteName(69)).toBe('A4');
  });

  it('snaps a sharp C to C in C major', () => {
    const c4 = 60;
    const sharp = c4 + 0.4;
    expect(snapMidi(sharp, 0, 'major')).toBe(60);
  });

  it('snaps toward D in C major instead of C#', () => {
    const between = 60 + 1.2;
    expect(snapMidi(between, 0, 'major')).toBe(62);
  });

  it('keeps C# in chromatic', () => {
    expect(snapMidi(61.1, 0, 'chromatic')).toBe(61);
  });

  it('corrects 466 Hz toward A or A# depending on scale', () => {
    const towardA = correctionRatio(466.16, 0, 'major');
    expect(towardA).toBeCloseTo(440 / 466.16, 3);
    const chromatic = correctionRatio(466.16, 0, 'chromatic');
    expect(chromatic).toBeCloseTo(1, 2);
  });

  it('returns unity for silence or out-of-range frequencies', () => {
    expect(correctionRatio(0, 0, 'major')).toBe(1);
    expect(correctionRatio(40, 0, 'major')).toBe(1);
    expect(correctionRatio(4000, 0, 'major')).toBe(1);
  });

  it('reports cents from a target', () => {
    expect(centsOff(440, 69)).toBeCloseTo(0, 5);
    expect(centsOff(midiToFreq(70), 69)).toBeCloseTo(100, 5);
  });
});
