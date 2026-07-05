import NetInfo from '@react-native-community/netinfo';
import { getConnectivitySnapshot } from '../network';

describe('getConnectivitySnapshot', () => {
  afterEach(() => {
    jest.clearAllMocks();
  });

  test('wifi with reachable internet reports connected + wifi', async () => {
    (NetInfo.fetch as jest.Mock).mockResolvedValueOnce({
      type: 'wifi',
      isConnected: true,
      isInternetReachable: true,
    });
    await expect(getConnectivitySnapshot()).resolves.toEqual({
      isConnected: true,
      isWifi: true,
    });
  });

  test('cellular connection reports connected but not wifi', async () => {
    (NetInfo.fetch as jest.Mock).mockResolvedValueOnce({
      type: 'cellular',
      isConnected: true,
      isInternetReachable: true,
    });
    await expect(getConnectivitySnapshot()).resolves.toEqual({
      isConnected: true,
      isWifi: false,
    });
  });

  test('isConnected true but isInternetReachable explicitly false is treated as offline', async () => {
    (NetInfo.fetch as jest.Mock).mockResolvedValueOnce({
      type: 'wifi',
      isConnected: true,
      isInternetReachable: false,
    });
    await expect(getConnectivitySnapshot()).resolves.toEqual({
      isConnected: false,
      isWifi: true,
    });
  });

  test('isInternetReachable of null (still probing) is not treated as offline', async () => {
    (NetInfo.fetch as jest.Mock).mockResolvedValueOnce({
      type: 'wifi',
      isConnected: true,
      isInternetReachable: null,
    });
    await expect(getConnectivitySnapshot()).resolves.toEqual({
      isConnected: true,
      isWifi: true,
    });
  });

  test('isConnected false reports offline regardless of type', async () => {
    (NetInfo.fetch as jest.Mock).mockResolvedValueOnce({
      type: 'none',
      isConnected: false,
      isInternetReachable: false,
    });
    await expect(getConnectivitySnapshot()).resolves.toEqual({
      isConnected: false,
      isWifi: false,
    });
  });
});
