import { assert } from '../lib/assert';

const MIN_SECONDS = 0.35;
const MAX_SECONDS = 3.2;
const MIN_DECAY = 1.6;
const MAX_DECAY = 3.8;

export function createImpulseResponse(
  ctx: AudioContext,
  roomSize: number,
): AudioBuffer {
  assert(ctx.sampleRate > 0, 'bad-sample-rate', 'AudioContext sample rate must be positive');
  const size = Math.min(1, Math.max(0, roomSize));
  const seconds = MIN_SECONDS + size * (MAX_SECONDS - MIN_SECONDS);
  const decay = MIN_DECAY + size * (MAX_DECAY - MIN_DECAY);
  const length = Math.max(1, Math.floor(ctx.sampleRate * seconds));
  const impulse = ctx.createBuffer(2, length, ctx.sampleRate);

  for (let channel = 0; channel < 2; channel += 1) {
    const data = impulse.getChannelData(channel);
    for (let i = 0; i < length; i += 1) {
      const envelope = (1 - i / length) ** decay;
      data[i] = (Math.random() * 2 - 1) * envelope;
    }
  }

  return impulse;
}
