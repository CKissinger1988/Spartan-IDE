import AsyncStorage from '@react-native-async-storage/async-storage';
import { QueuedDecision, QueuedDecisionKind } from '../types/queue';

// §69.5's offline-first review queue. Persists queued Approve/Reject
// decisions to on-device storage so they survive an app restart while
// offline, and can be replayed once connectivity returns.
//
// Honest boundary: there is no real sync client yet (no session-store
// backend exists — see the mobile repo's own README), so `replayQueue`
// below has nowhere real to send a decision. It only reports what's
// queued; actually delivering a queued decision, and handling the
// "artifact changed while this was in flight" conflict §69.5 requires,
// is unbuilt until a real backend exists to talk to.

const STORAGE_KEY = 'spartan.offlineDecisionQueue.v1';

async function readQueue(): Promise<QueuedDecision[]> {
  const raw = await AsyncStorage.getItem(STORAGE_KEY);
  if (!raw) return [];
  try {
    const parsed = JSON.parse(raw);
    return Array.isArray(parsed) ? parsed : [];
  } catch {
    return [];
  }
}

async function writeQueue(queue: QueuedDecision[]): Promise<void> {
  await AsyncStorage.setItem(STORAGE_KEY, JSON.stringify(queue));
}

export async function enqueueDecision(
  artifactId: string,
  decision: QueuedDecisionKind,
  seenArtifactUpdatedAt: string
): Promise<QueuedDecision> {
  const queue = await readQueue();
  const entry: QueuedDecision = {
    id: `queued-${Date.now()}`,
    artifactId,
    decision,
    seenArtifactUpdatedAt,
    queuedAt: new Date().toISOString(),
  };
  await writeQueue([...queue, entry]);
  return entry;
}

export async function getQueuedDecisions(): Promise<QueuedDecision[]> {
  return readQueue();
}

export async function clearQueue(): Promise<void> {
  await AsyncStorage.removeItem(STORAGE_KEY);
}

export interface ReplayResult {
  attempted: number;
  note: string;
}

// No-op stub, deliberately: there's nothing to sync to yet. Kept as a named
// call site so wiring in a real client later is a one-function change, not
// a redesign of where offline decisions live.
export async function replayQueue(): Promise<ReplayResult> {
  const queue = await readQueue();
  return {
    attempted: queue.length,
    note:
      'No sync backend exists yet — queued decisions remain queued. ' +
      'This call is a placeholder for when one does.',
  };
}
