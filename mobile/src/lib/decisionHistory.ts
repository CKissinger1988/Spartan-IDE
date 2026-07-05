import AsyncStorage from '@react-native-async-storage/async-storage';
import { Artifact } from '../types/domain';

// A local, persistent log of every Approve/Reject decision made across both
// places a decision can happen in this app (ArtifactReviewScreen's gated
// flow, and notificationActions.ts's direct low-stakes notification-button
// flow) -- mirrors offlineQueue.ts's AsyncStorage read/write/defensive-parse
// pattern. Honest boundary: this is a local audit log, not a synced one --
// no backend exists yet to reconcile it against.

const STORAGE_KEY = 'spartan.decisionHistory.v1';

export interface DecisionHistoryEntry {
  id: string;
  artifactId: string;
  artifactTitle: string;
  decision: 'approved' | 'rejected';
  decidedAt: string;
  queued: boolean;
}

async function readHistory(): Promise<DecisionHistoryEntry[]> {
  const raw = await AsyncStorage.getItem(STORAGE_KEY);
  if (!raw) return [];
  try {
    const parsed = JSON.parse(raw);
    return Array.isArray(parsed) ? parsed : [];
  } catch {
    return [];
  }
}

async function writeHistory(history: DecisionHistoryEntry[]): Promise<void> {
  await AsyncStorage.setItem(STORAGE_KEY, JSON.stringify(history));
}

export async function appendDecisionEntry(
  artifact: Artifact,
  decision: 'approved' | 'rejected',
  queued: boolean
): Promise<DecisionHistoryEntry> {
  const history = await readHistory();
  const entry: DecisionHistoryEntry = {
    id: `decision-${Date.now()}`,
    artifactId: artifact.id,
    artifactTitle: artifact.title,
    decision,
    decidedAt: new Date().toISOString(),
    queued,
  };
  await writeHistory([...history, entry]);
  return entry;
}

export async function getDecisionHistory(): Promise<DecisionHistoryEntry[]> {
  const history = await readHistory();
  return [...history].reverse();
}

export async function clearDecisionHistory(): Promise<void> {
  await AsyncStorage.removeItem(STORAGE_KEY);
}
