import * as LocalAuthentication from 'expo-local-authentication';
import { requireBiometricApproval } from '../biometricGate';

jest.mock('expo-local-authentication', () => ({
  hasHardwareAsync: jest.fn(),
  isEnrolledAsync: jest.fn(),
  authenticateAsync: jest.fn(),
}));

describe('requireBiometricApproval', () => {
  afterEach(() => {
    jest.clearAllMocks();
  });

  test('unavailable when the device has no biometric hardware at all', async () => {
    (LocalAuthentication.hasHardwareAsync as jest.Mock).mockResolvedValue(false);
    await expect(requireBiometricApproval('Confirm')).resolves.toEqual({
      outcome: 'unavailable',
      reason: 'No biometric hardware on this device.',
    });
    expect(LocalAuthentication.isEnrolledAsync).not.toHaveBeenCalled();
  });

  test('unavailable when hardware exists but nothing is enrolled', async () => {
    (LocalAuthentication.hasHardwareAsync as jest.Mock).mockResolvedValue(true);
    (LocalAuthentication.isEnrolledAsync as jest.Mock).mockResolvedValue(false);
    await expect(requireBiometricApproval('Confirm')).resolves.toEqual({
      outcome: 'unavailable',
      reason: 'No Face ID/fingerprint enrolled on this device.',
    });
    expect(LocalAuthentication.authenticateAsync).not.toHaveBeenCalled();
  });

  test('approved when authentication succeeds', async () => {
    (LocalAuthentication.hasHardwareAsync as jest.Mock).mockResolvedValue(true);
    (LocalAuthentication.isEnrolledAsync as jest.Mock).mockResolvedValue(true);
    (LocalAuthentication.authenticateAsync as jest.Mock).mockResolvedValue({ success: true });
    await expect(requireBiometricApproval('Confirm')).resolves.toEqual({ outcome: 'approved' });
  });

  test('denied when the OS prompt reports failure', async () => {
    (LocalAuthentication.hasHardwareAsync as jest.Mock).mockResolvedValue(true);
    (LocalAuthentication.isEnrolledAsync as jest.Mock).mockResolvedValue(true);
    (LocalAuthentication.authenticateAsync as jest.Mock).mockResolvedValue({ success: false });
    await expect(requireBiometricApproval('Confirm')).resolves.toEqual({ outcome: 'denied' });
  });

  test('passes the caller-supplied prompt message through with device fallback enabled', async () => {
    (LocalAuthentication.hasHardwareAsync as jest.Mock).mockResolvedValue(true);
    (LocalAuthentication.isEnrolledAsync as jest.Mock).mockResolvedValue(true);
    (LocalAuthentication.authenticateAsync as jest.Mock).mockResolvedValue({ success: true });
    await requireBiometricApproval('Confirm rejection');
    expect(LocalAuthentication.authenticateAsync).toHaveBeenCalledWith({
      promptMessage: 'Confirm rejection',
      disableDeviceFallback: false,
    });
  });
});
