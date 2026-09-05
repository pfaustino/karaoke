import { VoiceBooth, scaleFromSelect, type PitchFrame } from './audio/VoiceBooth';
import { centsOff } from './audio/pitch';

const micBtn = requiredEl('mic-btn', HTMLButtonElement);
const statusEl = requiredEl('status', HTMLParagraphElement);
const noteEl = requiredEl('note', HTMLElement);
const noteMetaEl = requiredEl('note-meta', HTMLElement);
const meterFill = requiredEl('meter-fill', HTMLElement);
const viz = requiredEl('viz', HTMLCanvasElement);
const trackToggle = requiredEl('track-toggle', HTMLButtonElement);
const trackFile = requiredEl('track-file', HTMLInputElement);

const booth = new VoiceBooth({
  onPitch: renderPitch,
  onError: (message) => setStatus(message, true),
});

const timeData = new Uint8Array(2048);

function requiredEl<T extends typeof HTMLElement>(id: string, ctor: T): InstanceType<T> {
  const node = document.getElementById(id);
  if (!(node instanceof ctor)) {
    throw new Error(`Missing #${id}`);
  }
  return node as InstanceType<T>;
}

function setStatus(text: string, isError = false): void {
  statusEl.textContent = text;
  statusEl.classList.toggle('error', isError);
}

function renderPitch(frame: PitchFrame): void {
  if (!frame.note) {
    noteEl.textContent = '—';
    noteMetaEl.textContent = booth.isLive() ? 'listening' : 'waiting';
    return;
  }
  noteEl.textContent = `${frame.note}${frame.octave}`;
  const cents = Math.round(centsOff(frame.freq, frame.targetMidi));
  const signed = cents > 0 ? `+${cents}` : `${cents}`;
  noteMetaEl.textContent = `${Math.round(frame.freq)} Hz · ${signed}¢`;
}

function bindReadouts(): void {
  document.querySelectorAll<HTMLInputElement>('input[type="range"]').forEach((input) => {
    const readout = document.querySelector(`[data-readout="${input.id}"]`);
    const paint = (): void => {
      if (readout) readout.textContent = Number(input.value).toFixed(2);
    };
    input.addEventListener('input', paint);
    paint();
  });
}

function applyControls(): void {
  booth.setVoiceGain(Number(requiredEl('voice-gain', HTMLInputElement).value));
  booth.setMasterGain(Number(requiredEl('master-gain', HTMLInputElement).value));
  booth.setAutotuneEnabled(requiredEl('tune-on', HTMLInputElement).checked);
  booth.setAutotuneAmount(Number(requiredEl('tune-amount', HTMLInputElement).value));
  booth.setAutotuneRetune(Number(requiredEl('tune-retune', HTMLInputElement).value));
  booth.setKey(Number(requiredEl('tune-key', HTMLSelectElement).value));
  booth.setScale(scaleFromSelect(requiredEl('tune-scale', HTMLSelectElement).value));
  booth.setReverbMix(Number(requiredEl('reverb-mix', HTMLInputElement).value));
  booth.setReverbSize(Number(requiredEl('reverb-size', HTMLInputElement).value));
  booth.setEchoMix(Number(requiredEl('echo-mix', HTMLInputElement).value));
  booth.setEchoTime(Number(requiredEl('echo-time', HTMLInputElement).value));
  booth.setTrackGain(Number(requiredEl('track-gain', HTMLInputElement).value));
}

function drawFrame(): void {
  const ctx = viz.getContext('2d');
  const analyser = booth.getAnalyser();
  if (!ctx) return;

  const { width, height } = viz;
  ctx.clearRect(0, 0, width, height);
  ctx.fillStyle = 'rgba(255, 209, 102, 0.08)';
  ctx.fillRect(0, height / 2, width, 1);

  if (analyser) {
    analyser.getByteTimeDomainData(timeData);
    ctx.beginPath();
    ctx.strokeStyle = '#ff2d95';
    ctx.lineWidth = 2;
    const slice = width / analyser.fftSize;
    let sum = 0;
    for (let i = 0; i < analyser.fftSize; i += 1) {
      const v = timeData[i] / 128 - 1;
      sum += v * v;
      const x = i * slice;
      const y = height / 2 + v * (height * 0.42);
      if (i === 0) ctx.moveTo(x, y);
      else ctx.lineTo(x, y);
    }
    ctx.stroke();
    const rms = Math.sqrt(sum / analyser.fftSize);
    meterFill.style.width = `${Math.min(100, rms * 280)}%`;
  }

  requestAnimationFrame(drawFrame);
}

async function toggleMic(): Promise<void> {
  micBtn.disabled = true;
  try {
    if (booth.isLive()) {
      await booth.stop();
      micBtn.textContent = 'Start microphone';
      micBtn.classList.remove('live');
      trackToggle.disabled = true;
      noteEl.textContent = '—';
      noteMetaEl.textContent = 'waiting';
      setStatus('Mic is off');
      return;
    }
    await booth.start();
    applyControls();
    micBtn.textContent = 'Stop microphone';
    micBtn.classList.add('live');
    trackToggle.disabled = !trackFile.files?.length;
    setStatus('Sing — you should hear yourself in the headphones');
  } catch {
    micBtn.textContent = 'Start microphone';
    micBtn.classList.remove('live');
  } finally {
    micBtn.disabled = false;
  }
}

micBtn.addEventListener('click', () => {
  void toggleMic();
});

['voice-gain', 'master-gain', 'tune-amount', 'tune-retune', 'reverb-mix', 'echo-mix', 'echo-time', 'track-gain'].forEach(
  (id) => {
    requiredEl(id, HTMLInputElement).addEventListener('input', applyControls);
  },
);
requiredEl('reverb-size', HTMLInputElement).addEventListener('change', applyControls);
requiredEl('tune-on', HTMLInputElement).addEventListener('change', applyControls);
requiredEl('tune-key', HTMLSelectElement).addEventListener('change', applyControls);
requiredEl('tune-scale', HTMLSelectElement).addEventListener('change', applyControls);

trackFile.addEventListener('change', () => {
  const file = trackFile.files?.[0];
  trackToggle.disabled = !file || !booth.isLive();
  if (!file || !booth.isLive()) {
    if (file && !booth.isLive()) setStatus('Start the microphone first, then load a track');
    return;
  }
  void booth.loadTrack(file).then(
    () => {
      trackToggle.textContent = 'Pause track';
      setStatus(`Playing ${file.name}`);
    },
    (err: unknown) => {
      const message = err instanceof Error ? err.message : 'Could not play that track';
      setStatus(message, true);
    },
  );
});

trackToggle.addEventListener('click', () => {
  booth.toggleTrack();
  trackToggle.textContent = booth.isTrackPlaying() ? 'Pause track' : 'Play track';
});

window.addEventListener('error', (event) => {
  setStatus(event.message || 'Unexpected error', true);
});
window.addEventListener('unhandledrejection', (event) => {
  const reason = event.reason;
  const message = reason instanceof Error ? reason.message : 'Unexpected error';
  setStatus(message, true);
});

bindReadouts();
requestAnimationFrame(drawFrame);
