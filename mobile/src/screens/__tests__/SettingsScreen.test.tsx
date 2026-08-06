import React from 'react';
import { act, fireEvent, render, waitFor } from '@testing-library/react-native';
import { SettingsScreen } from '../SettingsScreen';
import { getConnectivitySnapshot } from '../../lib/network';
import { requestNotificationPermission } from '../../lib/notifications';
import { schedulePreviewNotification } from '../../lib/notificationActions';
import { clearQueue, getQueuedDecisions, replayQueue } from '../../lib/offlineQueue';
import { getBackendPairingToken } from '../../lib/backendPairing';

const mockReconnect = jest.fn();
const mockUpdateEndpoint = jest.fn();

jest.mock('../../lib/backendContext', () => ({
  useBackendConnection: () => ({
    connecting: false,
    endpoint: 'http://192.168.1.20:4400',
    error: null,
    reconnect: mockReconnect,
    updateEndpoint: mockUpdateEndpoint,
  }),
}));

jest.mock('../../lib/backendPairing', () => ({
  getBackendPairingToken: jest.fn(),
}));

jest.mock('expo-camera', () => {
  const React = require('react');
  const { View } = require('react-native');
  return {
    CameraView: (props: any) => React.createElement(View, props),
    useCameraPermissions: () => [{ granted: true }, jest.fn()],
  };
});

jest.mock('@react-navigation/native', () => ({
  ...jest.requireActual('@react-navigation/native'),
  // Focus effects run after commit in the real navigator. Calling the
  // callback during render made this mock trigger SettingsScreen state
  // updates before mount under React 19, which is neither production
  // behavior nor a valid component test lifecycle.
  useFocusEffect: (cb: () => void) => {
    const React = require('react');
    React.useEffect(cb, [cb]);
  },
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
const mockGetBackendPairingToken = getBackendPairingToken as jest.Mock;

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
    mockUpdateEndpoint.mockResolvedValue(undefined);
    mockGetBackendPairingToken.mockResolvedValue(null);
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

  test('saves a configured Linux devserver endpoint and reconnects', async () => {
    const { findByText, getByTestId, getByText } = await renderScreen();
    await findByText('Connected to the Spartan devserver.');

    await act(async () => {
      fireEvent.changeText(getByTestId('backend-endpoint-input'), 'http://192.168.1.33:4400');
    });
    await act(async () => {
      fireEvent.changeText(getByTestId('backend-pairing-token-input'), 'paired-secret');
    });
    await act(async () => {
      fireEvent.press(getByText('Save and reconnect'));
    });

    await waitFor(() => {
      expect(mockUpdateEndpoint).toHaveBeenCalledWith('http://192.168.1.33:4400', 'paired-secret');
    });
  });

  test('retries the current Linux devserver connection', async () => {
    const { findByText, getByText } = await renderScreen();
    await findByText('Connected to the Spartan devserver.');
    fireEvent.press(getByText('Retry'));
    expect(mockReconnect).toHaveBeenCalledTimes(1);
  });

  test('imports a private-server pairing payload into the connection fields', async () => {
    const { findByText, getByTestId, getByText } = await renderScreen();
    await findByText('Connected to the Spartan devserver.');
    await act(async () => {
      fireEvent.changeText(
        getByTestId('pairing-payload-input'),
        'spartan://pair/v1?kind=private&endpoint=http%3A%2F%2F192.168.1.33%3A4400&pairing=paired'
      );
    });
    await act(async () => {
      fireEvent.press(getByText('Import pairing code'));
    });
    await waitFor(() => {
      expect(getByTestId('backend-endpoint-input').props.value).toBe('http://192.168.1.33:4400');
      expect(getByTestId('backend-pairing-token-input').props.value).toBe('paired');
    });
  });

  test('opens the QR scanner and imports a scanned private pairing payload', async () => {
    const { findByText, getByTestId, getByText } = await renderScreen();
    await findByText('Connected to the Spartan devserver.');
    await act(async () => {
      fireEvent.press(getByText('Scan pairing QR'));
    });
    await act(async () => {
      getByTestId('pairing-qr-camera').props.onBarcodeScanned({
        data: 'spartan://pair/v1?kind=private&endpoint=http%3A%2F%2F10.0.0.5%3A4400&pairing=scan',
      });
    });
    expect(getByTestId('backend-endpoint-input').props.value).toBe('http://10.0.0.5:4400');
    expect(getByTestId('backend-pairing-token-input').props.value).toBe('scan');
  });
});
