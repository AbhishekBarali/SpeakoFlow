/** One WebAudio output queue for every voice provider, beside the microphone's
 * WebRTC echo cancellation. Epochs also guard async decoding after Stop. */
export class ConversationAudio {
  private generation = 0;
  private epoch: number | null = null;
  private nextAt = 0;
  private sources = new Set<AudioBufferSourceNode>();
  private decoding: Promise<void> = Promise.resolve();
  private gain: GainNode;

  constructor(
    private context: AudioContext,
    private onPlaying: (playing: boolean) => void,
    volume = 1,
  ) {
    this.gain = context.createGain();
    this.gain.connect(context.destination);
    this.setVolume(volume);
  }

  /**
   * Apply a new playback volume, including to audio already queued.
   *
   * The gain node is shared by every source, so this takes effect mid-reply:
   * moving the voice-volume slider during a call should be audible in that call,
   * not in the next one.
   */
  setVolume(volume: number) {
    const level = Math.max(0, Math.min(1, volume));
    if (this.gain.gain.value === level) return;
    this.gain.gain.value = level;
  }

  begin(epoch: number) {
    if (this.epoch === epoch) return;
    this.stop();
    this.epoch = epoch;
  }

  enqueue(bytes: ArrayBuffer, epoch: number): Promise<void> {
    if (epoch !== this.epoch) return Promise.resolve();
    const generation = this.generation;
    const decoded = this.decoding.then(async () => {
      if (generation !== this.generation) return;
      const buffer = await this.context.decodeAudioData(bytes);
      if (generation !== this.generation || epoch !== this.epoch) return;
      const source = this.context.createBufferSource();
      source.buffer = buffer;
      source.connect(this.gain);
      this.sources.add(source);
      source.onended = () => {
        source.disconnect();
        this.sources.delete(source);
        if (generation === this.generation && this.sources.size === 0)
          this.onPlaying(false);
      };
      const at = Math.max(this.context.currentTime + 0.015, this.nextAt);
      this.nextAt = at + buffer.duration;
      source.start(at);
      this.onPlaying(true);
    });
    // A failed decode must not poison every subsequent utterance.
    this.decoding = decoded.catch(() => {});
    return decoded;
  }

  stop() {
    this.generation++;
    this.epoch = null;
    for (const source of this.sources) {
      source.onended = null;
      source.stop();
      source.disconnect();
    }
    this.sources.clear();
    this.nextAt = 0;
    this.decoding = Promise.resolve();
    this.onPlaying(false);
  }
}
