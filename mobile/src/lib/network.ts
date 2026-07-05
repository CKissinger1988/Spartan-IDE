import NetInfo from '@react-native-community/netinfo';

// §69.5's "Network-aware sync policy (v1, small)": heavier payloads default
// to Wi-Fi-only, with connectivity checked before deciding whether to queue
// a decision offline or (once a backend exists) send it immediately.

export interface ConnectivitySnapshot {
  isConnected: boolean;
  isWifi: boolean;
}

export async function getConnectivitySnapshot(): Promise<ConnectivitySnapshot> {
  const state = await NetInfo.fetch();
  return {
    isConnected: state.isConnected === true && state.isInternetReachable !== false,
    isWifi: state.type === 'wifi',
  };
}
