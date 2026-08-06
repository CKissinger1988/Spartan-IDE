import AsyncStorage from '@react-native-async-storage/async-storage';

const FIRST_RUN_COMPLETED_KEY = 'spartan.mobile.firstRunCompleted.v1';

export async function hasCompletedFirstRun(): Promise<boolean> {
  return (await AsyncStorage.getItem(FIRST_RUN_COMPLETED_KEY)) === '1';
}

export async function completeFirstRun(): Promise<void> {
  await AsyncStorage.setItem(FIRST_RUN_COMPLETED_KEY, '1');
}
