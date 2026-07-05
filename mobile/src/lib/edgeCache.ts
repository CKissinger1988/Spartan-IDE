import AsyncStorage from '@react-native-async-storage/async-storage';
import * as SecureStore from 'expo-secure-store';
import { Artifact, SessionThread } from '../types/domain';

// §69.5's "Edge-cached repo context (v1, small)": recently-viewed diffs,
// files, and artifact history cached on-device, scoped to what the user
// actually opened — not a full repo clone — so review context survives
// connectivity loss.
//
// Honest split, not a single "it's all encrypted" claim: SecureStore is
// Keychain/Keystore-backed but has a real per-key size ceiling (~2KB on
// Android) that a full diff patch can exceed. So only the small manifest
// (which artifacts were viewed, and when) lives in SecureStore; the actual
// diff/session content lives in AsyncStorage, which is app-sandboxed but
// not hardware-encryption-backed. A real "fully encrypted at rest" cache
// for large diff text would need chunking around SecureStore's limit (or
// an encrypted SQLite/file store) — deferred rather than faked here.

const MANIFEST_KEY = 'spartan.edgeCache.manifest.v1';
const ARTIFACT_PREFIX = 'spartan.edgeCache.artifact.';
const SESSION_LIST_KEY = 'spartan.edgeCache.sessionThreads.v1';

interface CacheManifestEntry {
  artifactId: string;
  cachedAt: string;
}

async function readManifest(): Promise<CacheManifestEntry[]> {
  const raw = await SecureStore.getItemAsync(MANIFEST_KEY);
  if (!raw) return [];
  try {
    const parsed = JSON.parse(raw);
    return Array.isArray(parsed) ? parsed : [];
  } catch {
    return [];
  }
}

async function writeManifest(entries: CacheManifestEntry[]): Promise<void> {
  await SecureStore.setItemAsync(MANIFEST_KEY, JSON.stringify(entries));
}

export async function cacheArtifact(artifact: Artifact): Promise<void> {
  await AsyncStorage.setItem(`${ARTIFACT_PREFIX}${artifact.id}`, JSON.stringify(artifact));

  const manifest = await readManifest();
  const withoutThis = manifest.filter((e) => e.artifactId !== artifact.id);
  await writeManifest([...withoutThis, { artifactId: artifact.id, cachedAt: new Date().toISOString() }]);
}

export async function getCachedArtifact(artifactId: string): Promise<Artifact | null> {
  const raw = await AsyncStorage.getItem(`${ARTIFACT_PREFIX}${artifactId}`);
  if (!raw) return null;
  try {
    return JSON.parse(raw) as Artifact;
  } catch {
    return null;
  }
}

export async function getCachedArtifactManifest(): Promise<CacheManifestEntry[]> {
  return readManifest();
}

export async function cacheSessionThreads(threads: SessionThread[]): Promise<void> {
  await AsyncStorage.setItem(SESSION_LIST_KEY, JSON.stringify(threads));
}

export async function getCachedSessionThreads(): Promise<SessionThread[] | null> {
  const raw = await AsyncStorage.getItem(SESSION_LIST_KEY);
  if (!raw) return null;
  try {
    const parsed = JSON.parse(raw);
    return Array.isArray(parsed) ? parsed : null;
  } catch {
    return null;
  }
}
