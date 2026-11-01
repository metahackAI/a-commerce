'use client';

// The composer's microphone (docs/api/chat.md §4): Astryx's dictation button, driven by our own
// recording and Muse Voice Transcribe instead of the browser's speech recognition. What was said
// lands in the draft; the person reads it and sends it like a typed message.

import {useEffect, useRef, useState} from 'react';
import {ChatDictationButton, useChatComposerContext} from '@astryxdesign/core/Chat';
import type {UseSpeechRecognitionReturn} from '@astryxdesign/core/Chat';

import {useAi} from '@/ai/AiProvider';
import {canRecord, record, transcribe, wordsOnScreen, type Recording} from '@/ai/voice';

type Status = {type: 'error' | 'warning'; message: string} | undefined;

const QUIET = {volume: 0, bands: [0, 0, 0, 0, 0]};

/** The button for a composer's `sendActions` and the status line it needs while it works. */
export function useVoiceInput(size: 'sm' | 'md' = 'md'): {button: React.ReactNode; status: Status} {
  const [problem, setProblem] = useState<string>();
  const [isWriting, setWriting] = useState(false);
  return {
    button: <VoiceButton size={size} onProblem={setProblem} onWriting={setWriting} />,
    status: problem
      ? {type: 'error', message: problem}
      : isWriting
        ? {type: 'warning', message: 'Writing down what you said…'}
        : undefined,
  };
}

function VoiceButton({
  size,
  onProblem,
  onWriting,
}: {
  size: 'sm' | 'md';
  onProblem: (p: string | undefined) => void;
  onWriting: (w: boolean) => void;
}) {
  const ai = useAi();
  const composer = useChatComposerContext();
  const draft = useRef('');
  draft.current = composer?.value ?? '';
  const recording = useRef<Recording | null>(null);
  const [isSupported, setSupported] = useState(false);
  const [isListening, setListening] = useState(false);
  const [level, setLevel] = useState(QUIET);

  // Known only in the browser; the server render shows no button.
  useEffect(() => setSupported(canRecord()), []);
  useEffect(() => () => recording.current?.cancel(), []);
  useEffect(() => {
    if (!isListening) return;
    let frame = 0;
    const tick = () => {
      if (recording.current) setLevel(recording.current.level());
      frame = requestAnimationFrame(tick);
    };
    frame = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(frame);
  }, [isListening]);

  const start = async () => {
    onProblem(undefined);
    try {
      recording.current = await record();
      setListening(true);
    } catch {
      onProblem('The microphone is not available. Allow it for this site, or type the message.');
    }
  };

  const stop = async () => {
    const r = recording.current;
    recording.current = null;
    setListening(false);
    setLevel(QUIET);
    if (!r) return;
    onWriting(true);
    try {
      const said = await transcribe(ai.surface, await r.stop(), wordsOnScreen());
      if (said) composer?.onChange(draft.current.trim() ? `${draft.current.trim()} ${said}` : said);
      else onProblem('Nothing was heard. Try again a little closer to the microphone.');
    } catch (e) {
      onProblem(e instanceof Error ? e.message : 'Voice input did not work. Type the message instead.');
    } finally {
      onWriting(false);
    }
  };

  const abort = () => {
    recording.current?.cancel();
    recording.current = null;
    setListening(false);
    setLevel(QUIET);
  };

  const dictation: UseSpeechRecognitionReturn = {
    isSupported,
    isListening,
    isSpeaking: isListening && level.volume > 0.15,
    volume: level.volume,
    bands: level.bands,
    rawBands: level.bands,
    interimTranscript: '',
    start: () => void start(),
    stop: () => void stop(),
    abort,
    toggle: () => void (isListening ? stop() : start()),
  };
  return <ChatDictationButton dictation={dictation} size={size} label={isListening ? 'Stop and write it down' : 'Speak instead of typing'} />;
}
