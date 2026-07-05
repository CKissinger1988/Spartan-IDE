// §69.5's "Offline-first review queue (v1 — this revises §69.1's scope, not
// just extends it)": a queued decision must record the artifact state the
// reviewer actually saw, so a later sync can detect "the artifact changed
// while this was in flight" instead of silently applying to a diff no one
// reviewed. seenArtifactUpdatedAt is that snapshot — it is NOT a promise
// that conflict detection is implemented end-to-end; there is no sync
// client yet to do the detecting. See src/lib/offlineQueue.ts.

export type QueuedDecisionKind = 'approved' | 'rejected';

export interface QueuedDecision {
  id: string;
  artifactId: string;
  decision: QueuedDecisionKind;
  seenArtifactUpdatedAt: string; // the artifact's createdAt at decision time
  queuedAt: string;
}
