const CLICK_DURATION_SECONDS = 0.012;
const CLICK_FILTER_FREQUENCY = 4_200;
const CLICK_FILTER_Q = 3.2;
const CLICK_BASE_GAIN = 0.22;

const CONFIRM_DURATION_SECONDS = 0.18;
const CONFIRM_START_FREQUENCY = 440;
const CONFIRM_END_FREQUENCY = 880;
const CONFIRM_BASE_GAIN = 0.2;

const ERROR_DURATION_SECONDS = 0.24;
const ERROR_START_FREQUENCY = 220;
const ERROR_END_FREQUENCY = 110;
const ERROR_BASE_GAIN = 0.18;

const ENVELOPE_FLOOR = 0.001;
const NOISE_BUFFER_SECONDS = 0.08;

export type SoundName = "click" | "confirm" | "error";

export interface SoundEngine {
  playClick(volume?: number): Promise<void>;
  playConfirm(volume?: number): Promise<void>;
  playError(volume?: number): Promise<void>;
  play(name: SoundName, volume?: number): Promise<void>;
  preload(): void;
}

function hasAudioSupport(): boolean {
  return typeof window !== "undefined" && typeof window.AudioContext !== "undefined";
}

function clampVolume(volume: number): number {
  if (!Number.isFinite(volume)) {
    return 0;
  }

  return Math.min(Math.max(volume, 0), 0.99);
}

class WebSoundEngine implements SoundEngine {
  private context: AudioContext | null = null;
  private clickBuffer: AudioBuffer | null = null;

  preload(): void {
    if (!hasAudioSupport()) {
      return;
    }

    this.getClickBuffer();
  }

  async playClick(volume = CLICK_BASE_GAIN): Promise<void> {
    const context = await this.resumeContext();
    if (!context) {
      return;
    }

    const source = context.createBufferSource();
    source.buffer = this.getClickBuffer();

    const filter = context.createBiquadFilter();
    filter.type = "bandpass";
    filter.frequency.setValueAtTime(CLICK_FILTER_FREQUENCY, context.currentTime);
    filter.Q.setValueAtTime(CLICK_FILTER_Q, context.currentTime);

    const gain = context.createGain();
    this.applyDecayEnvelope(gain.gain, context.currentTime, CLICK_DURATION_SECONDS, volume);

    source.connect(filter);
    filter.connect(gain);
    gain.connect(context.destination);

    this.scheduleCleanup(source, [source, filter, gain]);

    source.start(context.currentTime);
    source.stop(context.currentTime + CLICK_DURATION_SECONDS);
  }

  async playConfirm(volume = CONFIRM_BASE_GAIN): Promise<void> {
    const context = await this.resumeContext();
    if (!context) {
      return;
    }

    const oscillator = context.createOscillator();
    oscillator.type = "sine";
    oscillator.frequency.setValueAtTime(CONFIRM_START_FREQUENCY, context.currentTime);
    oscillator.frequency.exponentialRampToValueAtTime(
      CONFIRM_END_FREQUENCY,
      context.currentTime + CONFIRM_DURATION_SECONDS,
    );

    const gain = context.createGain();
    this.applyDecayEnvelope(gain.gain, context.currentTime, CONFIRM_DURATION_SECONDS, volume);

    oscillator.connect(gain);
    gain.connect(context.destination);

    this.scheduleCleanup(oscillator, [oscillator, gain]);

    oscillator.start(context.currentTime);
    oscillator.stop(context.currentTime + CONFIRM_DURATION_SECONDS);
  }

  async playError(volume = ERROR_BASE_GAIN): Promise<void> {
    const context = await this.resumeContext();
    if (!context) {
      return;
    }

    const oscillator = context.createOscillator();
    oscillator.type = "sawtooth";
    oscillator.frequency.setValueAtTime(ERROR_START_FREQUENCY, context.currentTime);
    oscillator.frequency.exponentialRampToValueAtTime(
      ERROR_END_FREQUENCY,
      context.currentTime + ERROR_DURATION_SECONDS,
    );

    const gain = context.createGain();
    this.applyDecayEnvelope(gain.gain, context.currentTime, ERROR_DURATION_SECONDS, volume);

    oscillator.connect(gain);
    gain.connect(context.destination);

    this.scheduleCleanup(oscillator, [oscillator, gain]);

    oscillator.start(context.currentTime);
    oscillator.stop(context.currentTime + ERROR_DURATION_SECONDS);
  }

  play(name: SoundName, volume?: number): Promise<void> {
    switch (name) {
      case "click":
        return this.playClick(volume);
      case "confirm":
        return this.playConfirm(volume);
      case "error":
        return this.playError(volume);
    }
  }

  private getContext(): AudioContext | null {
    if (!hasAudioSupport()) {
      return null;
    }

    if (!this.context) {
      this.context = new window.AudioContext();
    }

    return this.context;
  }

  private async resumeContext(): Promise<AudioContext | null> {
    const context = this.getContext();
    if (!context) {
      return null;
    }

    if (context.state === "suspended") {
      try {
        await context.resume();
      } catch {
        return null;
      }
    }

    return context.state === "running" ? context : null;
  }

  private getClickBuffer(): AudioBuffer {
    const context = this.getContext();
    if (!context) {
      throw new Error("AudioContext is unavailable.");
    }

    if (!this.clickBuffer) {
      const frameCount = Math.max(1, Math.floor(context.sampleRate * NOISE_BUFFER_SECONDS));
      const buffer = context.createBuffer(1, frameCount, context.sampleRate);
      const channel = buffer.getChannelData(0);

      for (let index = 0; index < frameCount; index += 1) {
        channel[index] = Math.random() * 2 - 1;
      }

      this.clickBuffer = buffer;
    }

    return this.clickBuffer;
  }

  private applyDecayEnvelope(
    param: AudioParam,
    startTime: number,
    durationSeconds: number,
    volume: number,
  ): void {
    const safeVolume = Math.max(clampVolume(volume), ENVELOPE_FLOOR);
    param.cancelScheduledValues(startTime);
    param.setValueAtTime(safeVolume, startTime);
    param.exponentialRampToValueAtTime(ENVELOPE_FLOOR, startTime + durationSeconds);
  }

  private scheduleCleanup(source: AudioScheduledSourceNode, nodes: AudioNode[]): void {
    source.onended = () => {
      for (const node of nodes) {
        node.disconnect();
      }
      source.onended = null;
    };
  }
}

const soundEngine = new WebSoundEngine();

export function getSoundEngine(): SoundEngine {
  return soundEngine;
}

export function preloadSoundEngine(): void {
  soundEngine.preload();
}

export async function playSound(name: SoundName, volume?: number): Promise<void> {
  await soundEngine.play(name, volume);
}
