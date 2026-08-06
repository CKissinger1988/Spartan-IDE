import AsyncStorage from '@react-native-async-storage/async-storage';

// The default remains useful for the web build and for an Android device
// reached through an explicit adb reverse tunnel. A physical phone on Wi-Fi
// must use the Linux machine's LAN address instead; Settings owns that
// explicit, persisted choice.
export const DEFAULT_BACKEND_ENDPOINT = 'http://127.0.0.1:4400';

const STORAGE_KEY = 'spartan.backendEndpoint.v1';

/**
 * Accept only a bare HTTP(S) server origin. Paths, credentials, query
 * strings, and fragments are deliberately rejected: the client appends the
 * fixed, security-sensitive session handoff path itself.
 */
export function normalizeBackendEndpoint(value: string): string | null {
  const trimmed = value.trim();
  if (!trimmed) return null;

  try {
    const url = new URL(trimmed);
    if ((url.protocol !== 'http:' && url.protocol !== 'https:') || !url.hostname) return null;
    if (url.username || url.password || url.pathname !== '/' || url.search || url.hash) return null;
    return url.origin;
  } catch {
    return null;
  }
}

export async function getBackendEndpoint(): Promise<string> {
  const stored = await AsyncStorage.getItem(STORAGE_KEY);
  return (stored && normalizeBackendEndpoint(stored)) ?? DEFAULT_BACKEND_ENDPOINT;
}

export async function setBackendEndpoint(value: string): Promise<string> {
  const endpoint = normalizeBackendEndpoint(value);
  if (!endpoint) {
    throw new Error('Enter a complete HTTP or HTTPS server address, such as http://192.168.1.20:4400.');
  }
  await AsyncStorage.setItem(STORAGE_KEY, endpoint);
  return endpoint;
}
