import { useCallback, useState } from 'react';
import {
  ExpoSpeechRecognitionModule,
  useSpeechRecognitionEvent,
} from 'expo-speech-recognition';

// §69.5's "Voice-to-task capture (v2)": on-device speech-to-text (the OS's
// own engine, not a cloud call) to dictate a new task into the Inbox —
// "deliberately narrower than §67.4's still-open 'live voice transcription'
// desktop gap: capture-and-submit, not a live conversational voice mode."
//
// Real constraint, stated plainly rather than glossed over: expo-speech-
// recognition is a third-party native module, not part of Expo Go's
// managed binary. It needs a custom dev client (EAS Build or a local
// Xcode/Android Studio prebuild) to actually run on a device — neither
// reachable in this environment. This hook is real, type-checked code
// against the package's documented API; it has not been exercised against
// an actual on-device speech engine, and `expo export`'s success proves
// only that the JS/TS glue resolves and bundles, not that recognition
// works. Don't take that bundle success as evidence beyond that.

export interface VoiceCaptureState {
  isListening: boolean;
  transcript: string;
  error: string | null;
}

export function useVoiceCapture() {
  const [state, setState] = useState<VoiceCaptureState>({
    isListening: false,
    transcript: '',
    error: null,
  });

  useSpeechRecognitionEvent('result', (event) => {
    const best = event.results[0]?.transcript ?? '';
    setState((prev) => ({ ...prev, transcript: best }));
  });

  useSpeechRecognitionEvent('error', (event) => {
    setState((prev) => ({ ...prev, error: event.message ?? event.error, isListening: false }));
  });

  useSpeechRecognitionEvent('end', () => {
    setState((prev) => ({ ...prev, isListening: false }));
  });

  const start = useCallback(async () => {
    const permission = await ExpoSpeechRecognitionModule.requestPermissionsAsync();
    if (!permission.granted) {
      setState((prev) => ({ ...prev, error: 'Microphone/speech permission not granted.' }));
      return;
    }

    setState({ isListening: true, transcript: '', error: null });
    ExpoSpeechRecognitionModule.start({
      lang: 'en-US',
      interimResults: true,
      continuous: false,
    });
  }, []);

  const stop = useCallback(() => {
    ExpoSpeechRecognitionModule.stop();
  }, []);

  const reset = useCallback(() => {
    setState({ isListening: false, transcript: '', error: null });
  }, []);

  const setTranscript = useCallback((transcript: string) => {
    setState((prev) => ({ ...prev, transcript }));
  }, []);

  return { ...state, start, stop, reset, setTranscript };
}
