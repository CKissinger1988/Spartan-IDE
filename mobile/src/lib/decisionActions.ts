import { requireBiometricApproval } from './biometricGate';
import { appendDecisionEntry } from './decisionHistory';
import { getConnectivitySnapshot } from './network';
import { enqueueDecision } from './offlineQueue';
import { Artifact } from '../types/domain';

// Shared by ArtifactReviewScreen (always gated) and the notification-action
// handler (§69.5's "Notification-surface actions" — gated only for
// destructive-class artifacts; low-stakes ones skip straight to recording
// the decision, by design, since that's the entire point of a notification
// action button). Kept as one function so the two call sites can't drift on
// what "recording a decision" actually means.

export type DecisionOutcome =
  | { status: 'recorded'; queued: boolean }
  | { status: 'denied' }
  | { status: 'biometric_unavailable'; reason: string; recorded: true; queued: boolean };

export interface RecordDecisionOptions {
  requireBiometric: boolean;
}

export async function recordDecision(
  artifact: Artifact,
  decision: 'approved' | 'rejected',
  options: RecordDecisionOptions
): Promise<DecisionOutcome> {
  if (options.requireBiometric) {
    const gate = await requireBiometricApproval(
      decision === 'approved' ? 'Confirm approval' : 'Confirm rejection'
    );

    if (gate.outcome === 'denied') {
      return { status: 'denied' };
    }

    if (gate.outcome === 'unavailable') {
      const queued = await queueOrRecord(artifact, decision);
      await appendDecisionEntry(artifact, decision, queued);
      return { status: 'biometric_unavailable', reason: gate.reason, recorded: true, queued };
    }
  }

  const queued = await queueOrRecord(artifact, decision);
  await appendDecisionEntry(artifact, decision, queued);
  return { status: 'recorded', queued };
}

async function queueOrRecord(
  artifact: Artifact,
  decision: 'approved' | 'rejected'
): Promise<boolean> {
  const { isConnected } = await getConnectivitySnapshot();
  if (!isConnected) {
    // §69.5: record "as of the state the reviewer actually saw"
    // (artifact.createdAt, since there's no separate updatedAt in this mock
    // model yet) so a later sync can detect the artifact changing
    // underneath a queued decision.
    await enqueueDecision(artifact.id, decision, artifact.createdAt);
    return true;
  }
  // Still local-only even when "connected" — no sync backend exists yet.
  return false;
}
