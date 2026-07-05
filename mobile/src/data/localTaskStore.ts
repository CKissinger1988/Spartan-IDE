import { SessionThread } from '../types/domain';

// A minimal external store for tasks added on-device (voice-dictated or
// typed) before any real session-store backend exists to submit them to.
// §69.5: "transcribed locally, submitted as an ordinary task thread when
// connectivity allows" — the "submitted" half is not built (no backend),
// so this only makes the new thread show up locally in the Inbox.

let localThreads: SessionThread[] = [];
const listeners = new Set<() => void>();

export function addLocalTask(title: string): SessionThread {
  const thread: SessionThread = {
    id: `local-${Date.now()}`,
    title,
    workspaceName: 'Dictated on-device',
    status: 'running',
    updatedAt: new Date().toISOString(),
    unreadCount: 0,
  };
  localThreads = [thread, ...localThreads];
  listeners.forEach((listener) => listener());
  return thread;
}

export function getLocalTasks(): SessionThread[] {
  return localThreads;
}

export function subscribeLocalTasks(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}
