import React from 'react';
import { act, fireEvent, render } from '@testing-library/react-native';
import { SettingsScreen } from '../SettingsScreen';
import { getConnectivitySnapshot } from '../../lib/network';
import { requestNotificationPermission } from '../../lib/notifications';
import { schedulePreviewNotification } from '../../lib/notificationActions';
import { clearQueue, getQueuedDecisions, replayQueue } from '../../lib/offlineQueue';

jest.mock('@react-navigation/native', () => ({
  ...jest.requireActual('@react-navigation/native'),
  useFocusEffect: (cb: () => void) => cb(),
}));

jest.mock('../../lib/network', () => ({
  getConnectivitySnapshot: jest.fn(),
}));

jest.mock('../../lib/notifications', () => ({
  requestNotificationPermission: jest.fn(),
}));

jest.mock('../../lib/notificationActions', () => ({
  schedulePreviewNotification: jest.fn(),
}));

jest.mock('../../lib/offlineQueue', () => ({
  getQueuedDecisions: jest.fn(),
  replayQueue: jest.fn(),
  clearQueue: jest.fn(),
}));

const mockGetConnectivitySnapshot = getConnectivitySnapshot as jest.Mock;
const mockRequestNotificationPermission = requestNotificationPermission as jest.Mock;
const mockSchedulePreviewNotification = schedulePreviewNotification as jest.Mock;
const mockGetQueuedDecisions = getQueuedDecisions as jest.Mock;
const mockReplayQueue = replayQueue as jest.Mock;
const mockClearQueue = clearQueue as jest.Mock;

async function renderScreen() {
  const navigation = { navigate: jest.fn() };
  const utils = await render(<SettingsScreen navigation={navigation as any} route={{} as any} />);
  return { navigation, ...utils };
}

describe('SettingsScreen', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockGetQueuedDecisions.mockResolvedValue([]);
    mockGetConnectivitySnapshot.mockResolvedValue({ isConnected: true, isWifi: true });
    mockSchedulePreviewNotification.mockResolvedValue(undefined);
    mockClearQueue.mockResolvedValue(undefined);
    mockReplayQueue.mockResolvedValue({ attempted: 0, note: 'No sync backend exists yet.' });
  });

  test('renders connected/Wi-Fi connectivity status', async () => {
    mockGetConnectivitySnapshot.mockResolvedValue({ isConnected: true, isWifi: true });
    const { findByText } = await renderScreen();
    expect(await findByText('Connected — Wi-Fi')).toBeTruthy();
  });

  test('renders connected/cellular connectivity status', async () => {
    mockGetConnectivitySnapshot.mockResolvedValue({ isConnected: true, isWifi: false });
    const { findByText } = await renderScreen();
    expect(await findByText('Connected — cellular')).toBeTruthy();
  });

  test('renders offline connectivity status', async () => {
    mockGetConnectivitySnapshot.mockResolvedValue({ isConnected: false, isWifi: false });
    const { findByText } = await renderScreen();
    expect(await findByText('Offline')).toBeTruthy();
  });

  test('toggling notifications on requests permission and reflects granted outcome', async () => {
    mockRequestNotificationPermission.mockResolvedValue({ granted: true });
    const { findByText, getByRole } = await renderScreen();
    await findByText('Connected — Wi-Fi');

    const toggle = getByRole('switch');
    expect(toggle.props.value).toBe(false);

    await act(async () => {
      fireEvent(toggle, 'valueChange', true);
    });

    expect(mockRequestNotificationPermission).toHaveBeenCalledTimes(1);
    expect(getByRole('switch').props.value).toBe(true);
  });

  test('toggling notifications on reflects a denied outcome and leaves the switch off', async () => {
    mockRequestNotificationPermission.mockResolvedValue({
      granted: false,
      reason: 'Permission was not granted.',
    });
    const { findByText, getByRole } = await renderScreen();
    await findByText('Connected — Wi-Fi');

    const toggle = getByRole('switch');

    await act(async () => {
      fireEvent(toggle, 'valueChange', true);
    });

    expect(mockRequestNotificationPermission).toHaveBeenCalledTimes(1);
    expect(getByRole('switch').props.value).toBe(false);
  });

  test('"Attempt sync" calls replayQueue', async () => {
    const { findByText, getByText } = await renderScreen();
    await findByText('Connected — Wi-Fi');

    await act(async () => {
      fireEvent.press(getByText('Attempt sync'));
    });

    expect(mockReplayQueue).toHaveBeenCalledTimes(1);
  });

  test('"Clear queue" calls clearQueue then re-fetches the queue', async () => {
    const { findByText, getByText } = await renderScreen();
    await findByText('Connected — Wi-Fi');

    const callsBeforeClear = mockGetQueuedDecisions.mock.calls.length;

    await act(async () => {
      fireEvent.press(getByText('Clear queue'));
    });

    expect(mockClearQueue).toHaveBeenCalledTimes(1);
    expect(mockGetQueuedDecisions.mock.calls.length).toBeGreaterThan(callsBeforeClear);
  });

  test('"View decision history" navigates to DecisionHistory', async () => {
    const { navigation, findByText, getByText } = await renderScreen();
    await findByText('Connected — Wi-Fi');

    fireEvent.press(getByText('View decision history'));

    expect(navigation.navigate).toHaveBeenCalledWith('DecisionHistory');
  });
});
