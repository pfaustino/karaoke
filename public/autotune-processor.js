/**
 * Real-time autotune: YIN pitch detect + overlap-add pitch shift.
 * Runs on the audio thread. Scale tables stay in sync with src/audio/pitch.ts.
 */

const SCALE_STEPS = [
  [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11],
  [0, 2, 4, 5, 7, 9, 11],
  [0, 2, 3, 5, 7, 8, 10],
];

const NOTE_NAMES = ['C', 'C#', 'D', 'D#', 'E', 'F', 'F#', 'G', 'G#', 'A', 'A#', 'B'];
const A4_HZ = 440;
const A4_MIDI = 69;
const MIN_HZ = 70;
const MAX_HZ = 1200;
const YIN_THRESHOLD = 0.15;
const PITCH_WINDOW = 1024;
const DOWNSAMPLE = 4;
const DETECT_HOP = 256;
const GRAIN = 512;
const DELAY = 2048;
const SEARCH_RADIUS = 12;
const MIN_RATIO = 0.5;
const MAX_RATIO = 2;
const GATE_RMS = 0.012;
const MSG_FRAMES = 8;

function freqToMidi(freq) {
  return A4_MIDI + 12 * Math.log2(freq / A4_HZ);
}

function midiToFreq(midi) {
  return A4_HZ * 2 ** ((midi - A4_MIDI) / 12);
}

function snapMidi(midi, key, scaleIndex) {
  const steps = SCALE_STEPS[scaleIndex] ?? SCALE_STEPS[0];
  const tonic = ((Math.round(key) % 12) + 12) % 12;
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

function yinPitch(samples, detectRate, diff) {
  const n = samples.length;
  const tauMin = Math.max(2, Math.floor(detectRate / MAX_HZ));
  const tauMax = Math.min(Math.floor(detectRate / MIN_HZ), n - 2, diff.length - 1);
  if (tauMax <= tauMin) return 0;

  for (let tau = 1; tau <= tauMax; tau += 1) {
    let sum = 0;
    const limit = n - tau;
    for (let i = 0; i < limit; i += 1) {
      const delta = samples[i] - samples[i + tau];
      sum += delta * delta;
    }
    diff[tau] = sum;
  }

  diff[0] = 1;
  let running = 0;
  for (let tau = 1; tau <= tauMax; tau += 1) {
    running += diff[tau];
    diff[tau] = (diff[tau] * tau) / running;
  }

  let tauEstimate = 0;
  for (let tau = tauMin; tau <= tauMax; tau += 1) {
    if (diff[tau] >= YIN_THRESHOLD) continue;
    let local = tau;
    const walkLimit = Math.min(tauMax, tau + 16);
    for (let step = tau + 1; step <= walkLimit; step += 1) {
      if (diff[step] >= diff[local]) break;
      local = step;
    }
    tauEstimate = local;
    break;
  }

  if (tauEstimate === 0 || diff[tauEstimate] >= YIN_THRESHOLD) return 0;

  const prev = tauEstimate > 1 ? diff[tauEstimate - 1] : diff[tauEstimate];
  const next = tauEstimate < tauMax ? diff[tauEstimate + 1] : diff[tauEstimate];
  const denom = 2 * (2 * diff[tauEstimate] - next - prev);
  const shift = Math.abs(denom) > 1e-9 ? (next - prev) / denom : 0;
  const betterTau = tauEstimate + shift;
  if (betterTau <= 0) return 0;
  return detectRate / betterTau;
}

function makeHann(size) {
  const window = new Float32Array(size);
  for (let i = 0; i < size; i += 1) {
    window[i] = 0.5 * (1 - Math.cos((2 * Math.PI * i) / size));
  }
  return window;
}

function readCubic(buffer, pos, length) {
  const wrapped = ((pos % length) + length) % length;
  const i = Math.floor(wrapped);
  const f = wrapped - i;
  const ym1 = buffer[((i - 1) % length + length) % length];
  const y0 = buffer[i];
  const y1 = buffer[(i + 1) % length];
  const y2 = buffer[(i + 2) % length];
  const c0 = y0;
  const c1 = 0.5 * (y1 - ym1);
  const c2 = ym1 - 2.5 * y0 + 2 * y1 - 0.5 * y2;
  const c3 = 0.5 * (y2 - ym1) + 1.5 * (y0 - y1);
  return ((c3 * f + c2) * f + c1) * f + c0;
}

class AutotuneProcessor extends AudioWorkletProcessor {
  static get parameterDescriptors() {
    return [
      { name: 'amount', defaultValue: 0.85, minValue: 0, maxValue: 1 },
      { name: 'retune', defaultValue: 0.55, minValue: 0.02, maxValue: 1 },
      { name: 'key', defaultValue: 0, minValue: 0, maxValue: 11 },
      { name: 'scale', defaultValue: 1, minValue: 0, maxValue: 2 },
      { name: 'bypass', defaultValue: 0, minValue: 0, maxValue: 1 },
    ];
  }

  constructor() {
    super();
    this.ring = new Float32Array(PITCH_WINDOW * DOWNSAMPLE);
    this.ringPos = 0;
    this.ringFilled = false;
    this.samplesSinceDetect = 0;
    this.down = new Float32Array(PITCH_WINDOW);
    this.diff = new Float32Array(Math.floor(PITCH_WINDOW / 2));
    this.delay = new Float32Array(DELAY);
    this.writePos = 0;
    this.grains = [
      { pos: 0, age: 0 },
      { pos: GRAIN / 2, age: GRAIN / 2 },
    ];
    this.hann = makeHann(GRAIN);
    this.ratio = 1;
    this.freq = 0;
    this.targetMidi = 0;
    this.framesUntilMsg = 0;
  }

  detectFromRing() {
    if (!this.ringFilled) return;
    let energy = 0;
    const len = this.ring.length;
    for (let i = 0; i < PITCH_WINDOW; i += 1) {
      const sample = this.ring[(this.ringPos + i * DOWNSAMPLE) % len];
      this.down[i] = sample;
      energy += sample * sample;
    }
    const rms = Math.sqrt(energy / PITCH_WINDOW);
    if (rms < GATE_RMS) {
      this.freq = 0;
      return;
    }
    const detected = yinPitch(this.down, sampleRate / DOWNSAMPLE, this.diff);
    this.freq = detected;
  }

  shiftSample(input, ratio) {
    this.delay[this.writePos] = input;
    let mix = 0;
    let weight = 0;
    for (let g = 0; g < this.grains.length; g += 1) {
      const grain = this.grains[g];
      const amp = this.hann[grain.age];
      mix += readCubic(this.delay, grain.pos, DELAY) * amp;
      weight += amp;
      grain.pos += ratio;
      grain.age += 1;
      if (grain.age >= GRAIN) {
        grain.pos = (this.writePos - GRAIN + DELAY) % DELAY;
        grain.age = 0;
      }
    }
    this.writePos = (this.writePos + 1) % DELAY;
    return weight > 1e-6 ? mix / weight : 0;
  }

  process(inputs, outputs, parameters) {
    const input = inputs[0]?.[0];
    const output = outputs[0]?.[0];
    if (!input || !output) return true;

    const amount = parameters.amount;
    const retune = parameters.retune;
    const key = parameters.key;
    const scale = parameters.scale;
    const bypass = parameters.bypass;
    const amountK = amount.length === 1;
    const retuneK = retune.length === 1;
    const keyK = key.length === 1;
    const scaleK = scale.length === 1;
    const bypassK = bypass.length === 1;

    for (let i = 0; i < input.length; i += 1) {
      const x = input[i];
      this.ring[this.ringPos] = x;
      this.ringPos += 1;
      if (this.ringPos >= this.ring.length) {
        this.ringPos = 0;
        this.ringFilled = true;
      }
      this.samplesSinceDetect += 1;
      if (this.samplesSinceDetect >= DETECT_HOP) {
        this.samplesSinceDetect = 0;
        this.detectFromRing();
      }

      const bypassVal = bypassK ? bypass[0] : bypass[i];
      if (bypassVal >= 0.5) {
        output[i] = x;
        continue;
      }

      const keyVal = keyK ? key[0] : key[i];
      const scaleVal = Math.round(scaleK ? scale[0] : scale[i]);
      const amountVal = amountK ? amount[0] : amount[i];
      const retuneVal = retuneK ? retune[0] : retune[i];

      let targetRatio = 1;
      if (this.freq >= MIN_HZ && this.freq <= MAX_HZ) {
        const midi = freqToMidi(this.freq);
        this.targetMidi = snapMidi(midi, keyVal, scaleVal);
        targetRatio = midiToFreq(this.targetMidi) / this.freq;
        targetRatio = Math.min(MAX_RATIO, Math.max(MIN_RATIO, targetRatio));
      }

      const slew = Math.min(1, Math.max(0.02, retuneVal));
      this.ratio += (targetRatio - this.ratio) * slew;
      const wet = this.shiftSample(x, this.ratio);
      output[i] = x * (1 - amountVal) + wet * amountVal;
    }

    this.framesUntilMsg -= 1;
    if (this.framesUntilMsg <= 0) {
      this.framesUntilMsg = MSG_FRAMES;
      const midi = this.freq > 0 ? freqToMidi(this.freq) : 0;
      const noteIndex = this.targetMidi ? ((Math.round(this.targetMidi) % 12) + 12) % 12 : 0;
      this.port.postMessage({
        freq: this.freq,
        midi,
        targetMidi: this.targetMidi,
        note: this.freq > 0 ? NOTE_NAMES[noteIndex] : '',
        octave: this.freq > 0 ? Math.floor(Math.round(this.targetMidi) / 12) - 1 : 0,
      });
    }

    return true;
  }
}

registerProcessor('autotune-processor', AutotuneProcessor);
