import AsyncStorage from '@react-native-async-storage/async-storage';
import {
  DEFAULT_BACKEND_ENDPOINT,
  getBackendEndpoint,
  normalizeBackendEndpoint,
  setBackendEndpoint,
} from '../backendEndpoint';

describe('backendEndpoint', () => {
  beforeEach(async () => {
    await AsyncStorage.clear();
  });

  test('defaults to the loopback devserver endpoint', async () => {
    await expect(getBackendEndpoint()).resolves.toBe(DEFAULT_BACKEND_ENDPOINT);
  });

  test('normalizes a LAN server origin and persists it', async () => {
    expect(normalizeBackendEndpoint(' http://192.168.1.20:4400/ ')).toBe(
      'http://192.168.1.20:4400'
    );
    await expect(setBackendEndpoint('http://192.168.1.20:4400/')).resolves.toBe(
      'http://192.168.1.20:4400'
    );
    await expect(getBackendEndpoint()).resolves.toBe('http://192.168.1.20:4400');
  });

  test.each([
    '',
    '192.168.1.20:4400',
    'ftp://192.168.1.20:4400',
    'http://user:secret@192.168.1.20:4400',
    'http://192.168.1.20:4400/__spartan/session',
    'http://192.168.1.20:4400/?token=secret',
  ])('rejects an unsafe or incomplete endpoint: %s', (value) => {
    expect(normalizeBackendEndpoint(value)).toBeNull();
  });
});
