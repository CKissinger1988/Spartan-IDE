import * as SecureStore from 'expo-secure-store';

// The pairing secret is a capability for obtaining the short-lived WebSocket
// token from a private Spartan devserver. Unlike the endpoint, it belongs in
// Keychain/Keystore-backed SecureStore rather than AsyncStorage.
const STORAGE_KEY = 'spartan.backendPairingToken.v1';

export async function getBackendPairingToken(): Promise<string | null> {
  return SecureStore.getItemAsync(STORAGE_KEY);
}

export async function setBackendPairingToken(value: string): Promise<void> {
  const token = value.trim();
  if (!token) {
    await SecureStore.deleteItemAsync(STORAGE_KEY);
    return;
  }
  await SecureStore.setItemAsync(STORAGE_KEY, token);
}
