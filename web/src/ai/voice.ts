// Voice input for the composer (docs/api/chat.md §4). The browser records the microphone, turns
// the recording into the 16 kHz mono 16-bit WAV the recognizer takes, and posts it with the words
// on the screen as hints. The agent service hands it to Muse Voice Transcribe and answers with
// the text, which goes back into the composer for the person to read and send.

import {csrfToken} from '@/lib/api';

import type {Surface} from './types';

const SAMPLE_RATE = 16000;

/** A running recording: the live level for the button, and `stop` for the WAV. */
export interface Recording {
  /** Microphone level 0–1 and five frequency bands, read as often as the button redraws. */
  level: () => {volume: number; bands: number[]};
  stop: () => Promise<Blob>;
  cancel: () => void;
}

export function canRecord(): boolean {
  return (
    typeof window !== 'undefined' &&
    typeof window.MediaRecorder !== 'undefined' &&
    typeof window.AudioContext !== 'undefined' &&
    Boolean(navigator.mediaDevices?.getUserMedia)
  );
}

export async function record(): Promise<Recording> {
  const stream = await navigator.mediaDevices.getUserMedia({audio: {channelCount: 1, echoCancellation: true, noiseSuppression: true}});
  const audio = new AudioContext();
  const analyser = audio.createAnalyser();
  analyser.fftSize = 256;
  audio.createMediaStreamSource(stream).connect(analyser);
  const recorder = new MediaRecorder(stream);
  const chunks: Blob[] = [];
  recorder.ondataavailable = e => {
    if (e.data.size > 0) chunks.push(e.data);
  };
  recorder.start();

  const release = () => {
    stream.getTracks().forEach(t => t.stop());
    void audio.close();
  };
  const freq = new Uint8Array(analyser.frequencyBinCount);
  return {
    level: () => {
      analyser.getByteFrequencyData(freq);
      const bands = [0, 1, 2, 3, 4].map(b => {
        const from = Math.floor((b * freq.length) / 10);
        const to = Math.floor(((b + 1) * freq.length) / 10);
        let sum = 0;
        for (let i = from; i < to; i++) sum += freq[i];
        return Math.min(1, sum / Math.max(1, to - from) / 160);
      });
      return {volume: Math.max(...bands), bands};
    },
    stop: () =>
      new Promise<Blob>((resolve, reject) => {
        recorder.onstop = () => {
          release();
          const blob = new Blob(chunks, {type: recorder.mimeType});
          toWav(blob).then(resolve, reject);
        };
        recorder.stop();
      }),
    cancel: () => {
      recorder.onstop = release;
      if (recorder.state !== 'inactive') recorder.stop();
      else release();
    },
  };
}

/** Decodes what the browser recorded (webm, mp4, …) and writes it as 16 kHz mono 16-bit WAV. */
async function toWav(recorded: Blob): Promise<Blob> {
  const decoded = await new AudioContext().decodeAudioData(await recorded.arrayBuffer());
  const frames = Math.ceil(decoded.duration * SAMPLE_RATE);
  const offline = new OfflineAudioContext(1, Math.max(1, frames), SAMPLE_RATE);
  const source = offline.createBufferSource();
  source.buffer = decoded;
  source.connect(offline.destination);
  source.start();
  const pcm = (await offline.startRendering()).getChannelData(0);

  const out = new DataView(new ArrayBuffer(44 + pcm.length * 2));
  const text = (at: number, s: string) => [...s].forEach((c, i) => out.setUint8(at + i, c.charCodeAt(0)));
  text(0, 'RIFF');
  out.setUint32(4, 36 + pcm.length * 2, true);
  text(8, 'WAVE');
  text(12, 'fmt ');
  out.setUint32(16, 16, true); // fmt chunk size
  out.setUint16(20, 1, true); // PCM
  out.setUint16(22, 1, true); // mono
  out.setUint32(24, SAMPLE_RATE, true);
  out.setUint32(28, SAMPLE_RATE * 2, true); // bytes per second
  out.setUint16(32, 2, true); // bytes per frame
  out.setUint16(34, 16, true); // bits per sample
  text(36, 'data');
  out.setUint32(40, pcm.length * 2, true);
  pcm.forEach((v, i) => {
    const s = Math.max(-1, Math.min(1, v));
    out.setInt16(44 + i * 2, s < 0 ? s * 0x8000 : s * 0x7fff, true);
  });
  return new Blob([out.buffer], {type: 'audio/wav'});
}

/** Separators inside a target label ("Wool overcoat · $289.00 · stock 24", "Size: XL"). */
const LABEL_PARTS = /\s+·\s+|:\s+|,\s+|\s+→\s+|\s+—\s+/;

/**
 * The names on the screen (AI targets' labels), as spelling hints: each label cut at its
 * separators, keeping the parts that are names (product, color, size) rather than numbers or ids.
 */
export function wordsOnScreen(): string[] {
  if (typeof document === 'undefined') return [];
  const parts = [...document.querySelectorAll<HTMLElement>('[data-acom-label]')]
    .flatMap(el => (el.dataset.acomLabel ?? '').split(LABEL_PARTS))
    .map(p => p.trim())
    .filter(p => p.length > 0 && p.length <= 30 && !/[\d_]/.test(p));
  return [...new Set(parts)];
}

/** Sends a recording and returns what was said. Throws with a sentence a person can read. */
export async function transcribe(surface: Surface, wav: Blob, keywords: string[]): Promise<string> {
  const form = new FormData();
  form.append('audio', wav, 'speech.wav');
  form.append('keywords', JSON.stringify(keywords));
  const csrf = csrfToken();
  const res = await fetch(`/api/chat/${surface}/transcriptions`, {
    method: 'POST',
    headers: csrf ? {'x-csrf-token': csrf} : {},
    credentials: 'same-origin',
    body: form,
  });
  const body = (await res.json().catch(() => null)) as {transcript?: string; error?: {message?: string}} | null;
  if (!res.ok) throw new Error(body?.error?.message ?? `Voice input is not available (${res.status}).`);
  return body?.transcript ?? '';
}
