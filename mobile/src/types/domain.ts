// Data shapes this app expects from the shared session store (§8, §50.1 of the
// desktop spec at ../../docs/architecture-spec.md §69).
// No backend exists yet — see src/data/mockData.ts for the placeholder source.
// These types are the contract this UI is built against, not a finalized API.

export type SessionStatus = 'running' | 'review' | 'done';

export interface SessionThread {
  id: string;
  title: string;
  workspaceName: string;
  status: SessionStatus;
  updatedAt: string; // ISO 8601
  unreadCount: number;
}

export type ArtifactKind = 'diff_card' | 'implementation_plan' | 'walkthrough';

export interface ArtifactFileChange {
  path: string;
  additions: number;
  deletions: number;
  patch: string; // unified diff text, syntax-highlighted read-only in DiffViewer
}

export interface Artifact {
  id: string;
  sessionId: string;
  kind: ArtifactKind;
  title: string;
  summary: string;
  createdAt: string;
  files: ArtifactFileChange[];
  approved: boolean | null; // null = pending decision
  // §69.5's "Notification-surface actions": low-stakes artifacts can be
  // approved/rejected directly from the push notification; anything in the
  // destructive class always requires opening the app and passing the
  // biometric gate. This is a placeholder classification — a real one
  // belongs on the backend, not decided client-side once one exists.
  lowStakes: boolean;
}

export interface ChatMessage {
  id: string;
  sessionId: string;
  author: 'user' | 'leo';
  text: string;
  createdAt: string;
}

// §69.1's "Artifact commenting" — leaving feedback on a plan or diff, the
// same async-review use case the whole app exists for.
export interface ArtifactComment {
  id: string;
  artifactId: string;
  filePath: string | null; // null = comment on the artifact as a whole
  author: string;
  text: string;
  createdAt: string;
}

// §69.5's "Camera capture into artifacts": photograph a whiteboard sketch
// or an error message on another machine's screen, attached to a session
// as context — the same way §14's log-line/stack-trace attachment already
// works on desktop. ocrText is always null here: on-device OCR needs a
// native module (e.g. ML Kit text recognition) that requires a custom dev
// client, which can't be built or run in this environment — see
// src/lib/imageCapture.ts and the README before assuming it's wired up.
export interface ImageAttachment {
  id: string;
  sessionId: string;
  uri: string;
  caption: string | null;
  ocrText: string | null;
  createdAt: string;
}
