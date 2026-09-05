import { assert } from '../lib/assert';
import { createImpulseResponse } from './reverb';
import type { ScaleId } from './pitch';
import { SCALE_IDS } from './pitch';

export type PitchFrame = {
  freq: number;
  midi: number;
  targetMidi: number;
  note: string;
  octave: number;
};

export type BoothHandlers = {
  onPitch: (frame: PitchFrame) => void;
  onError: (message: string) => void;
};

const SCALE_INDEX: Record<ScaleId, number> = {
  chromatic: 0,
  major: 1,
  minor: 2,
};

export class VoiceBooth {
  private ctx: AudioContext | null = null;
  private stream: MediaStream | null = null;
  private source: MediaStreamAudioSourceNode | null = null;
  private highpass: BiquadFilterNode | null = null;
  private inputGain: GainNode | null = null;
  private compressor: DynamicsCompressorNode | null = null;
  private autotune: AudioWorkletNode | null = null;
  private dry: GainNode | null = null;
  private convolver: ConvolverNode | null = null;
  private reverbWet: GainNode | null = null;
  private delay: DelayNode | null = null;
  private delayFeedback: GainNode | null = null;
  private echoWet: GainNode | null = null;
  private voiceMix: GainNode | null = null;
  private master: GainNode | null = null;
  private analyser: AnalyserNode | null = null;
  private trackEl: HTMLAudioElement | null = null;
  private trackSource: MediaElementAudioSourceNode | null = null;
  private trackGain: GainNode | null = null;
  private trackUrl: string | null = null;
  private roomSize = 0.55;
  private live = false;

  constructor(private readonly handlers: BoothHandlers) {}

  isLive(): boolean {
    return this.live;
  }

  getAnalyser(): AnalyserNode | null {
    return this.analyser;
  }

  async start(): Promise<void> {
    if (this.live) return;

    const ctx = new AudioContext({ latencyHint: 0.005 });
    this.ctx = ctx;

    try {
      await ctx.resume();
      await ctx.audioWorklet.addModule(`${import.meta.env.BASE_URL}autotune-processor.js`);
      const stream = await this.openMic(ctx);
      this.stream = stream;
      this.buildGraph(ctx, stream);
      this.live = true;
    } catch (err) {
      await this.teardown();
      const message = err instanceof Error ? err.message : 'Could not start the microphone';
      this.handlers.onError(this.friendlyMicError(message));
      throw err;
    }
  }

  async stop(): Promise<void> {
    await this.teardown();
  }

  setVoiceGain(value: number): void {
    this.setGain(this.inputGain, value);
  }

  setMasterGain(value: number): void {
    this.setGain(this.master, value);
  }

  setTrackGain(value: number): void {
    this.setGain(this.trackGain, value);
  }

  setAutotuneEnabled(on: boolean): void {
    const param = this.autotune?.parameters.get('bypass');
    if (!param) return;
    param.value = on ? 0 : 1;
  }

  setAutotuneAmount(value: number): void {
    const param = this.autotune?.parameters.get('amount');
    if (!param) return;
    param.value = clamp01(value);
  }

  setAutotuneRetune(value: number): void {
    const param = this.autotune?.parameters.get('retune');
    if (!param) return;
    param.value = Math.min(1, Math.max(0.02, value));
  }

  setKey(semitone: number): void {
    const param = this.autotune?.parameters.get('key');
    if (!param) return;
    param.value = ((Math.round(semitone) % 12) + 12) % 12;
  }

  setScale(scale: ScaleId): void {
    const param = this.autotune?.parameters.get('scale');
    if (!param) return;
    param.value = SCALE_INDEX[scale];
  }

  setReverbMix(value: number): void {
    this.setGain(this.reverbWet, clamp01(value) * 0.85);
  }

  setReverbSize(value: number): void {
    if (!this.ctx || !this.convolver) return;
    this.roomSize = clamp01(value);
    this.convolver.buffer = createImpulseResponse(this.ctx, this.roomSize);
  }

  setEchoMix(value: number): void {
    this.setGain(this.echoWet, clamp01(value) * 0.55);
  }

  setEchoTime(seconds: number): void {
    if (!this.delay) return;
    this.delay.delayTime.value = Math.min(0.9, Math.max(0.08, seconds));
  }

  async loadTrack(file: File): Promise<void> {
    assert(this.ctx, 'no-audio', 'Start the microphone before loading a track');
    if (this.trackUrl) URL.revokeObjectURL(this.trackUrl);
    const url = URL.createObjectURL(file);
    this.trackUrl = url;
    if (!this.trackEl) {
      this.trackEl = new Audio();
      this.trackEl.crossOrigin = 'anonymous';
      this.trackEl.loop = false;
      this.trackSource = this.ctx.createMediaElementSource(this.trackEl);
      this.trackGain = this.ctx.createGain();
      this.trackGain.gain.value = 0.7;
      this.trackSource.connect(this.trackGain);
      this.trackGain.connect(this.ctx.destination);
    }
    this.trackEl.pause();
    this.trackEl.src = url;
    await this.trackEl.play();
  }

  toggleTrack(): void {
    if (!this.trackEl) return;
    if (this.trackEl.paused) {
      void this.trackEl.play();
      return;
    }
    this.trackEl.pause();
  }

  isTrackPlaying(): boolean {
    return Boolean(this.trackEl && !this.trackEl.paused);
  }

  private async openMic(ctx: AudioContext): Promise<MediaStream> {
    const raw = {
      channelCount: 1,
      echoCancellation: false,
      noiseSuppression: false,
      autoGainControl: false,
    };
    try {
      const lowLatency: MediaTrackConstraints = {
        ...raw,
        sampleRate: ctx.sampleRate,
      };
      Object.assign(lowLatency, { latency: 0.005 });
      return await navigator.mediaDevices.getUserMedia({
        audio: lowLatency,
        video: false,
      });
    } catch {
      return await navigator.mediaDevices.getUserMedia({
        audio: raw,
        video: false,
      });
    }
  }

  private buildGraph(ctx: AudioContext, stream: MediaStream): void {
    const source = ctx.createMediaStreamSource(stream);
    const highpass = ctx.createBiquadFilter();
    highpass.type = 'highpass';
    highpass.frequency.value = 80;

    const inputGain = ctx.createGain();
    inputGain.gain.value = 1.4;

    const compressor = ctx.createDynamicsCompressor();
    compressor.threshold.value = -22;
    compressor.knee.value = 14;
    compressor.ratio.value = 4;
    compressor.attack.value = 0.004;
    compressor.release.value = 0.16;

    const autotune = new AudioWorkletNode(ctx, 'autotune-processor', {
      numberOfInputs: 1,
      numberOfOutputs: 1,
      outputChannelCount: [1],
    });
    autotune.port.onmessage = (event: MessageEvent<PitchFrame>) => {
      this.handlers.onPitch(event.data);
    };

    const dry = ctx.createGain();
    dry.gain.value = 1;

    const convolver = ctx.createConvolver();
    convolver.buffer = createImpulseResponse(ctx, this.roomSize);
    const reverbWet = ctx.createGain();
    reverbWet.gain.value = 0.28;

    const delay = ctx.createDelay(1.2);
    delay.delayTime.value = 0.26;
    const delayFeedback = ctx.createGain();
    delayFeedback.gain.value = 0.28;
    const echoWet = ctx.createGain();
    echoWet.gain.value = 0.12;

    const voiceMix = ctx.createGain();
    const master = ctx.createGain();
    master.gain.value = 0.95;

    const analyser = ctx.createAnalyser();
    analyser.fftSize = 2048;
    analyser.smoothingTimeConstant = 0.7;

    source.connect(highpass);
    highpass.connect(inputGain);
    inputGain.connect(compressor);
    compressor.connect(autotune);
    autotune.connect(dry);
    autotune.connect(convolver);
    autotune.connect(delay);
    delay.connect(delayFeedback);
    delayFeedback.connect(delay);
    dry.connect(voiceMix);
    convolver.connect(reverbWet);
    reverbWet.connect(voiceMix);
    delay.connect(echoWet);
    echoWet.connect(voiceMix);
    voiceMix.connect(master);
    master.connect(analyser);
    master.connect(ctx.destination);

    this.source = source;
    this.highpass = highpass;
    this.inputGain = inputGain;
    this.compressor = compressor;
    this.autotune = autotune;
    this.dry = dry;
    this.convolver = convolver;
    this.reverbWet = reverbWet;
    this.delay = delay;
    this.delayFeedback = delayFeedback;
    this.echoWet = echoWet;
    this.voiceMix = voiceMix;
    this.master = master;
    this.analyser = analyser;
  }

  private setGain(node: GainNode | null, value: number): void {
    if (!node) return;
    node.gain.value = Math.max(0, value);
  }

  private friendlyMicError(raw: string): string {
    const lower = raw.toLowerCase();
    if (lower.includes('denied') || lower.includes('not allowed')) {
      return 'Microphone permission was blocked. Allow mic access and try again.';
    }
    if (lower.includes('not found') || lower.includes('device')) {
      return 'No microphone was found. Plug one in and reload.';
    }
    return raw;
  }

  private async teardown(): Promise<void> {
    this.live = false;
    this.stream?.getTracks().forEach((track) => track.stop());
    this.stream = null;
    this.source?.disconnect();
    this.highpass?.disconnect();
    this.inputGain?.disconnect();
    this.compressor?.disconnect();
    this.autotune?.disconnect();
    this.dry?.disconnect();
    this.convolver?.disconnect();
    this.reverbWet?.disconnect();
    this.delay?.disconnect();
    this.delayFeedback?.disconnect();
    this.echoWet?.disconnect();
    this.voiceMix?.disconnect();
    this.master?.disconnect();
    this.analyser?.disconnect();
    this.source = null;
    this.highpass = null;
    this.inputGain = null;
    this.compressor = null;
    this.autotune = null;
    this.dry = null;
    this.convolver = null;
    this.reverbWet = null;
    this.delay = null;
    this.delayFeedback = null;
    this.echoWet = null;
    this.voiceMix = null;
    this.master = null;
    this.analyser = null;
    this.trackEl?.pause();
    this.trackSource?.disconnect();
    this.trackGain?.disconnect();
    this.trackEl = null;
    this.trackSource = null;
    this.trackGain = null;
    if (this.trackUrl) {
      URL.revokeObjectURL(this.trackUrl);
      this.trackUrl = null;
    }
    if (this.ctx) {
      await this.ctx.close();
      this.ctx = null;
    }
  }
}

export function scaleFromSelect(value: string): ScaleId {
  return SCALE_IDS.includes(value as ScaleId) ? (value as ScaleId) : 'major';
}

function clamp01(value: number): number {
  return Math.min(1, Math.max(0, value));
}
