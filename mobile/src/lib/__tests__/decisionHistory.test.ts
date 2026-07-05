import AsyncStorage from '@react-native-async-storage/async-storage';
import {
  appendDecisionEntry,
  getDecisionHistory,
  clearDecisionHistory,
} from '../decisionHistory';
import { Artifact } from '../../types/domain';

// Mirrors decisionHistory.ts's private STORAGE_KEY -- needed here only to
// inject malformed state directly, exercising the module's own parse-error
// fallback rather than its public write path.
const STORAGE_KEY = 'spartan.decisionHistory.v1';

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

describe('decisionHistory', () => {
  beforeEach(async () => {
    await AsyncStorage.clear();
  });

  test('appendDecisionEntry persists the decision', async () => {
    const entry = await appendDecisionEntry(artifact, 'approved', false);
    expect(entry.artifactId).toBe('artifact-1');
    expect(entry.artifactTitle).toBe('Refactor auth middleware');
    expect(entry.decision).toBe('approved');
    expect(entry.queued).toBe(false);
    await expect(getDecisionHistory()).resolves.toEqual([entry]);
  });

  test('multiple entries accumulate and are returned newest-first', async () => {
    const first = await appendDecisionEntry(artifact, 'approved', false);
    const second = await appendDecisionEntry(artifact, 'rejected', true);
    await expect(getDecisionHistory()).resolves.toEqual([second, first]);
  });

  test('getDecisionHistory is empty when nothing has been recorded', async () => {
    await expect(getDecisionHistory()).resolves.toEqual([]);
  });

  test('clearDecisionHistory empties a previously non-empty history', async () => {
    await appendDecisionEntry(artifact, 'approved', false);
    await clearDecisionHistory();
    await expect(getDecisionHistory()).resolves.toEqual([]);
  });

  test('corrupted stored JSON is treated as an empty history, not thrown', async () => {
    await AsyncStorage.setItem(STORAGE_KEY, 'not valid json{{{');
    await expect(getDecisionHistory()).resolves.toEqual([]);
  });

  test('a stored value that parses but is not an array is treated as empty', async () => {
    await AsyncStorage.setItem(STORAGE_KEY, JSON.stringify({ not: 'an array' }));
    await expect(getDecisionHistory()).resolves.toEqual([]);
  });
});
