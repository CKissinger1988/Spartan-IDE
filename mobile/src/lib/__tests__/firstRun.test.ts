import AsyncStorage from '@react-native-async-storage/async-storage';
import { completeFirstRun, hasCompletedFirstRun } from '../firstRun';

describe('mobile first-run state', () => {
  beforeEach(() => jest.clearAllMocks());

  test('only reports completion for the explicit persisted marker', async () => {
    (AsyncStorage.getItem as jest.Mock).mockResolvedValueOnce(null).mockResolvedValueOnce('1');
    await expect(hasCompletedFirstRun()).resolves.toBe(false);
    await expect(hasCompletedFirstRun()).resolves.toBe(true);
  });

  test('persists completion under a versioned key', async () => {
    (AsyncStorage.setItem as jest.Mock).mockResolvedValue(undefined);
    await completeFirstRun();
    expect(AsyncStorage.setItem).toHaveBeenCalledWith('spartan.mobile.firstRunCompleted.v1', '1');
  });
});
