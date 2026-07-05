import * as LocalAuthentication from 'expo-local-authentication';

// §69.5's "Biometric approval gating (v1)": destructive-action approvals
// (§36.4's approval matrix, in the desktop spec) require Face ID/fingerprint
// on mobile. This binds the highest-stakes decisions to hardware-backed
// presence in a way a desktop keyboard can't — an addition to §69.3's
// posture, not a relaxation of it.
//
// Not verified on real hardware: no device/emulator was reachable when this
// was written, so this has only been exercised via expo-local-authentication's
// documented API, not an actual Face ID/fingerprint prompt.
export type BiometricGateResult =
  | { outcome: 'approved' }
  | { outcome: 'denied' }
  | { outcome: 'unavailable'; reason: string };

export async function requireBiometricApproval(
  promptMessage: string
): Promise<BiometricGateResult> {
  const hasHardware = await LocalAuthentication.hasHardwareAsync();
  if (!hasHardware) {
    return { outcome: 'unavailable', reason: 'No biometric hardware on this device.' };
  }

  const isEnrolled = await LocalAuthentication.isEnrolledAsync();
  if (!isEnrolled) {
    return {
      outcome: 'unavailable',
      reason: 'No Face ID/fingerprint enrolled on this device.',
    };
  }

  const result = await LocalAuthentication.authenticateAsync({
    promptMessage,
    disableDeviceFallback: false,
  });

  return result.success ? { outcome: 'approved' } : { outcome: 'denied' };
}
