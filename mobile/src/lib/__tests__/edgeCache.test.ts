import AsyncStorage from '@react-native-async-storage/async-storage';
import * as SecureStore from 'expo-secure-store';
import {
  cacheArtifact,
  getCachedArtifact,
  getCachedArtifactManifest,
  cacheSessionThreads,
  getCachedSessionThreads,
} from '../edgeCache';
import { Artifact, SessionThread } from '../../types/domain';

// expo-secure-store has no first-party jest mock (unlike AsyncStorage/NetInfo),
// so this stands in a minimal in-memory Keychain/Keystore substitute -- real
// enough to exercise edgeCache's manifest read/write path, not a claim about
// SecureStore's actual native behavior.
jest.mock('expo-secure-store', () => {
  let store: Record<string, string> = {};
  return {
    getItemAsync: jest.fn(async (key: string) => store[key] ?? null),
    setItemAsync: jest.fn(async (key: string, value: string) => {
      store[key] = value;
    }),
    __reset: () => {
      store = {};
    },
  };
});

const artifact: Artifact = {
  id: 'artifact-1',
  sessionId: 'session-1',
  kind: 'diff_card',
  title: 'Refactor auth middleware',
  summary: 'summary',
  createdAt: '2026-01-01T00:00:00Z',
  files: [{ path: 'src/auth.ts', additions: 10, deletions: 2, patch: '@@ ... @@' }],
  approved: null,
  lowStakes: false,
};

const threads: SessionThread[] = [
  {
    id: 'session-1',
    title: 'Refactor auth middleware',
    workspaceName: 'spartan-ide',
    status: 'review',
    updatedAt: '2026-01-01T00:00:00Z',
    unreadCount: 1,
  },
];

describe('edgeCache', () => {
  beforeEach(async () => {
    await AsyncStorage.clear();
    (SecureStore as unknown as { __reset: () => void }).__reset();
  });

  test('cacheArtifact stores full content, retrievable by id', async () => {
    await cacheArtifact(artifact);
    await expect(getCachedArtifact(artifact.id)).resolves.toEqual(artifact);
  });

  test('cacheArtifact records a manifest entry alongside the content', async () => {
    await cacheArtifact(artifact);
    const manifest = await getCachedArtifactManifest();
    expect(manifest).toHaveLength(1);
    expect(manifest[0].artifactId).toBe(artifact.id);
  });

  test('re-caching the same artifact replaces its manifest entry rather than duplicating it', async () => {
    await cacheArtifact(artifact);
    await cacheArtifact(artifact);
    const manifest = await getCachedArtifactManifest();
    expect(manifest).toHaveLength(1);
  });

  test('caching a second, distinct artifact adds to the manifest rather than replacing it', async () => {
    await cacheArtifact(artifact);
    await cacheArtifact({ ...artifact, id: 'artifact-2' });
    const manifest = await getCachedArtifactManifest();
    const ids = manifest.map((entry) => entry.artifactId).sort();
    expect(ids).toEqual(['artifact-1', 'artifact-2']);
  });

  test('getCachedArtifact returns null for an artifact that was never cached', async () => {
    await expect(getCachedArtifact('never-cached')).resolves.toBeNull();
  });

  test('cacheSessionThreads / getCachedSessionThreads round-trip the thread list', async () => {
    await cacheSessionThreads(threads);
    await expect(getCachedSessionThreads()).resolves.toEqual(threads);
  });

  test('getCachedSessionThreads returns null, not an empty array, when nothing was ever cached', async () => {
    await expect(getCachedSessionThreads()).resolves.toBeNull();
  });

  test('corrupted manifest JSON in SecureStore is treated as empty, not thrown', async () => {
    await SecureStore.setItemAsync('spartan.edgeCache.manifest.v1', 'not valid json{{{');
    await expect(getCachedArtifactManifest()).resolves.toEqual([]);
  });
});
