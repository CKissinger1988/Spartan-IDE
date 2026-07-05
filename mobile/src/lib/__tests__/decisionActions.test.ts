import { recordDecision } from '../decisionActions';
import { requireBiometricApproval } from '../biometricGate';
import { getConnectivitySnapshot } from '../network';
import { enqueueDecision } from '../offlineQueue';
import { Artifact } from '../../types/domain';

jest.mock('../biometricGate');
jest.mock('../network');
jest.mock('../offlineQueue');

const artifact: Artifact = {
  id: 'artifact-1',
  sessionId: 'session-1',
  kind: 'diff_card',
  title: 'Refactor auth middleware',
  summary: 'summary',
  createdAt: '2026-01-01T00:00:00Z',
  files: [],
  approved: null,
  lowStakes: false,
};

describe('recordDecision', () => {
  afterEach(() => {
    jest.clearAllMocks();
  });

  test('records immediately, skipping the biometric gate, when not required', async () => {
    (getConnectivitySnapshot as jest.Mock).mockResolvedValue({ isConnected: true, isWifi: true });
    const outcome = await recordDecision(artifact, 'approved', { requireBiometric: false });
    expect(outcome).toEqual({ status: 'recorded', queued: false });
    expect(requireBiometricApproval).not.toHaveBeenCalled();
  });

  test('queues the decision with the seen artifact state when offline', async () => {
    (getConnectivitySnapshot as jest.Mock).mockResolvedValue({
      isConnected: false,
      isWifi: false,
    });
    (enqueueDecision as jest.Mock).mockResolvedValue({});
    const outcome = await recordDecision(artifact, 'rejected', { requireBiometric: false });
    expect(outcome).toEqual({ status: 'recorded', queued: true });
    expect(enqueueDecision).toHaveBeenCalledWith('artifact-1', 'rejected', artifact.createdAt);
  });

  test('denies the decision outright when the biometric gate is denied -- nothing is recorded', async () => {
    (requireBiometricApproval as jest.Mock).mockResolvedValue({ outcome: 'denied' });
    const outcome = await recordDecision(artifact, 'approved', { requireBiometric: true });
    expect(outcome).toEqual({ status: 'denied' });
    expect(getConnectivitySnapshot).not.toHaveBeenCalled();
    expect(enqueueDecision).not.toHaveBeenCalled();
  });

  test('records anyway, but flags it, when biometric hardware is unavailable', async () => {
    (requireBiometricApproval as jest.Mock).mockResolvedValue({
      outcome: 'unavailable',
      reason: 'No biometric hardware on this device.',
    });
    (getConnectivitySnapshot as jest.Mock).mockResolvedValue({ isConnected: true, isWifi: true });
    const outcome = await recordDecision(artifact, 'approved', { requireBiometric: true });
    expect(outcome).toEqual({
      status: 'biometric_unavailable',
      reason: 'No biometric hardware on this device.',
      recorded: true,
      queued: false,
    });
  });

  test('records normally once biometric approval succeeds', async () => {
    (requireBiometricApproval as jest.Mock).mockResolvedValue({ outcome: 'approved' });
    (getConnectivitySnapshot as jest.Mock).mockResolvedValue({ isConnected: true, isWifi: true });
    const outcome = await recordDecision(artifact, 'approved', { requireBiometric: true });
    expect(outcome).toEqual({ status: 'recorded', queued: false });
  });

  test('a "connected" decision is still local-only -- no sync backend exists to actually reach', async () => {
    (getConnectivitySnapshot as jest.Mock).mockResolvedValue({ isConnected: true, isWifi: true });
    const outcome = await recordDecision(artifact, 'approved', { requireBiometric: false });
    expect(outcome).toEqual({ status: 'recorded', queued: false });
    expect(enqueueDecision).not.toHaveBeenCalled();
  });
});
