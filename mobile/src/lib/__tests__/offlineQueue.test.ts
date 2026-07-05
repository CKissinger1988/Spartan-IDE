import AsyncStorage from '@react-native-async-storage/async-storage';
import { enqueueDecision, getQueuedDecisions, clearQueue, replayQueue } from '../offlineQueue';

// Mirrors offlineQueue.ts's private STORAGE_KEY -- needed here only to
// inject malformed state directly, exercising the module's own parse-error
// fallback rather than its public write path.
const STORAGE_KEY = 'spartan.offlineDecisionQueue.v1';

describe('offlineQueue', () => {
  beforeEach(async () => {
    await AsyncStorage.clear();
  });

  test('enqueueDecision persists the artifact state the reviewer actually saw', async () => {
    const entry = await enqueueDecision('artifact-1', 'approved', '2026-01-01T00:00:00Z');
    expect(entry.artifactId).toBe('artifact-1');
    expect(entry.decision).toBe('approved');
    expect(entry.seenArtifactUpdatedAt).toBe('2026-01-01T00:00:00Z');
    await expect(getQueuedDecisions()).resolves.toEqual([entry]);
  });

  test('multiple queued decisions accumulate in insertion order', async () => {
    const first = await enqueueDecision('a1', 'approved', 't1');
    const second = await enqueueDecision('a2', 'rejected', 't2');
    await expect(getQueuedDecisions()).resolves.toEqual([first, second]);
  });

  test('getQueuedDecisions is empty when nothing has been queued', async () => {
    await expect(getQueuedDecisions()).resolves.toEqual([]);
  });

  test('clearQueue empties a previously non-empty queue', async () => {
    await enqueueDecision('a1', 'approved', 't1');
    await clearQueue();
    await expect(getQueuedDecisions()).resolves.toEqual([]);
  });

  test('corrupted stored JSON is treated as an empty queue, not thrown', async () => {
    await AsyncStorage.setItem(STORAGE_KEY, 'not valid json{{{');
    await expect(getQueuedDecisions()).resolves.toEqual([]);
  });

  test('a stored value that parses but is not an array is treated as empty', async () => {
    await AsyncStorage.setItem(STORAGE_KEY, JSON.stringify({ not: 'an array' }));
    await expect(getQueuedDecisions()).resolves.toEqual([]);
  });

  test('replayQueue reports what is queued without pretending to sync it anywhere', async () => {
    await enqueueDecision('a1', 'approved', 't1');
    await enqueueDecision('a2', 'rejected', 't2');
    const result = await replayQueue();
    expect(result.attempted).toBe(2);
    expect(result.note).toMatch(/No sync backend exists yet/);
    // A no-op stub must not have silently drained the queue it just reported on.
    await expect(getQueuedDecisions()).resolves.toHaveLength(2);
  });
});
