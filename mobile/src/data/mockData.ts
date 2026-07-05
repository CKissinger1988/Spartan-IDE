// Placeholder local data standing in for the real session-store API (§69.1:
// "read from the same session store, not a separate mobile-only backend").
// Replace with a real client once that API exists — nothing here should be
// mistaken for a working backend integration.

import { Artifact, ArtifactComment, ChatMessage, SessionThread } from '../types/domain';

export const mockSessionThreads: SessionThread[] = [
  {
    id: 'sess-1',
    title: 'Fix checkout race condition',
    workspaceName: 'storefront-api',
    status: 'review',
    updatedAt: '2026-07-04T14:02:00Z',
    unreadCount: 1,
  },
  {
    id: 'sess-2',
    title: 'Add retry backoff to webhook sender',
    workspaceName: 'storefront-api',
    status: 'running',
    updatedAt: '2026-07-04T13:40:00Z',
    unreadCount: 0,
  },
  {
    id: 'sess-3',
    title: 'Migrate settings panel to new theme tokens',
    workspaceName: 'spartan-ide-desktop',
    status: 'done',
    updatedAt: '2026-07-03T19:15:00Z',
    unreadCount: 0,
  },
];

export const mockArtifacts: Artifact[] = [
  {
    id: 'art-1',
    sessionId: 'sess-1',
    kind: 'diff_card',
    title: 'Guard checkout total against concurrent stock updates',
    summary:
      'Adds a compare-and-swap check before committing the order total, closing the race ' +
      'window between stock read and order write.',
    createdAt: '2026-07-04T14:01:00Z',
    approved: null,
    lowStakes: false,
    files: [
      {
        path: 'src/checkout/commitOrder.ts',
        additions: 14,
        deletions: 3,
        patch:
          '@@ -22,7 +22,10 @@ export async function commitOrder(order: PendingOrder) {\n' +
          '-  await db.orders.insert(order);\n' +
          '+  const latestStock = await db.stock.read(order.sku);\n' +
          '+  if (latestStock.version !== order.stockVersionSeen) {\n' +
          "+    throw new StockChangedError(order.sku);\n" +
          '+  }\n' +
          '+  await db.orders.insert(order);\n',
      },
    ],
  },
  {
    id: 'art-2',
    sessionId: 'sess-2',
    kind: 'diff_card',
    title: 'Fix a typo in the webhook retry log message',
    summary: 'Corrects "recieved" to "received" in the retry-exhausted log line. No logic change.',
    createdAt: '2026-07-04T13:41:00Z',
    approved: null,
    lowStakes: true,
    files: [
      {
        path: 'src/webhooks/sender.ts',
        additions: 1,
        deletions: 1,
        patch:
          '@@ -88,7 +88,7 @@ async function giveUp(event: WebhookEvent) {\n' +
          "-  log.warn(`recieved final failure for ${event.id}`);\n" +
          "+  log.warn(`received final failure for ${event.id}`);\n",
      },
    ],
  },
];

export const mockArtifactComments: ArtifactComment[] = [
  {
    id: 'cmt-1',
    artifactId: 'art-1',
    filePath: 'src/checkout/commitOrder.ts',
    author: 'You',
    text: 'Should this throw StockChangedError or retry once before giving up?',
    createdAt: '2026-07-04T14:05:00Z',
  },
];

export const mockChatMessages: ChatMessage[] = [
  {
    id: 'msg-1',
    sessionId: 'sess-1',
    author: 'user',
    text: 'What triggered this fix?',
    createdAt: '2026-07-04T13:58:00Z',
  },
  {
    id: 'msg-2',
    sessionId: 'sess-1',
    author: 'leo',
    text:
      'A support ticket describing double-decremented stock during a flash sale. I traced it ' +
      'to a read-then-write gap in commitOrder with no version check. Diff card is ready for review.',
    createdAt: '2026-07-04T13:59:30Z',
  },
];
