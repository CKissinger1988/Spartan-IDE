import * as ImagePicker from 'expo-image-picker';

// §69.5's "Camera capture into artifacts (v2)". Uses expo-image-picker,
// which runs in Expo Go/managed workflow with no custom native module
// needed — unlike the on-device speech recognition and OCR packages this
// repo intentionally does NOT depend on (see src/lib/voiceCapture.ts and
// this module's own honest note about ocrText). External Content Fetch
// Gating (§50.2, desktop spec) applies unchanged: a captured image
// containing a URL never triggers an automatic fetch here or anywhere else
// in this app.

export interface CaptureResult {
  cancelled: boolean;
  uri: string | null;
}

async function requestCameraPermission(): Promise<boolean> {
  const { status } = await ImagePicker.requestCameraPermissionsAsync();
  return status === 'granted';
}

export async function captureFromCamera(): Promise<CaptureResult> {
  const granted = await requestCameraPermission();
  if (!granted) return { cancelled: true, uri: null };

  const result = await ImagePicker.launchCameraAsync({
    mediaTypes: ['images'],
    quality: 0.7,
  });

  if (result.canceled || result.assets.length === 0) {
    return { cancelled: true, uri: null };
  }
  return { cancelled: false, uri: result.assets[0].uri };
}

export async function pickFromLibrary(): Promise<CaptureResult> {
  const { status } = await ImagePicker.requestMediaLibraryPermissionsAsync();
  if (status !== 'granted') return { cancelled: true, uri: null };

  const result = await ImagePicker.launchImageLibraryAsync({
    mediaTypes: ['images'],
    quality: 0.7,
  });

  if (result.canceled || result.assets.length === 0) {
    return { cancelled: true, uri: null };
  }
  return { cancelled: false, uri: result.assets[0].uri };
}
