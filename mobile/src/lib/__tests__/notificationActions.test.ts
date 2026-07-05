import * as Notifications from 'expo-notifications';
import {
  categoryForArtifact,
  addNotificationResponseListener,
} from '../notificationActions';
import { recordDecision } from '../decisionActions';
import { navigationRef } from '../../navigation/navigationRef';
import { Artifact } from '../../types/domain';

jest.mock('expo-notifications', () => ({
  setNotificationCategoryAsync: jest.fn(),
  scheduleNotificationAsync: jest.fn(),
  addNotificationResponseReceivedListener: jest.fn(),
}));
jest.mock('../decisionActions');
jest.mock('../../navigation/navigationRef', () => ({
  navigationRef: { isReady: jest.fn(), navigate: jest.fn() },
}));

const lowStakesArtifact: Artifact = {
  id: 'artifact-low',
  sessionId: 'session-1',
  kind: 'diff_card',
  title: 'Tweak README wording',
  summary: 'summary',
  createdAt: '2026-01-01T00:00:00Z',
  files: [],
  approved: null,
  lowStakes: true,
};

const destructiveArtifact: Artifact = {
  ...lowStakesArtifact,
  id: 'artifact-destructive',
  lowStakes: false,
};

describe('categoryForArtifact', () => {
  test('low-stakes artifacts get the actionable category', () => {
    expect(categoryForArtifact(lowStakesArtifact)).toBe('review-actionable-low-stakes');
  });

  test('destructive-class artifacts get the no-action-buttons category', () => {
    expect(categoryForArtifact(destructiveArtifact)).toBe('review-actionable-destructive');
  });
});

describe('addNotificationResponseListener', () => {
  afterEach(() => {
    jest.clearAllMocks();
  });

  function fireResponse(response: unknown) {
    const listener = (Notifications.addNotificationResponseReceivedListener as jest.Mock).mock
      .calls[0][0];
    return listener(response);
  }

  test('approve action on a low-stakes artifact records the decision without biometric gating', async () => {
    const lookup = jest.fn().mockReturnValue(lowStakesArtifact);
    addNotificationResponseListener(lookup);

    await fireResponse({
      notification: { request: { content: { data: { artifactId: 'artifact-low' } } } },
      actionIdentifier: 'approve',
    });

    expect(recordDecision).toHaveBeenCalledWith(lowStakesArtifact, 'approved', {
      requireBiometric: false,
    });
  });

  test('reject action maps to a rejected decision', async () => {
    const lookup = jest.fn().mockReturnValue(lowStakesArtifact);
    addNotificationResponseListener(lookup);

    await fireResponse({
      notification: { request: { content: { data: { artifactId: 'artifact-low' } } } },
      actionIdentifier: 'reject',
    });

    expect(recordDecision).toHaveBeenCalledWith(lowStakesArtifact, 'rejected', {
      requireBiometric: false,
    });
  });

  test('an approve/reject action button is never honored for a destructive-class artifact', async () => {
    const lookup = jest.fn().mockReturnValue(destructiveArtifact);
    addNotificationResponseListener(lookup);

    await fireResponse({
      notification: { request: { content: { data: { artifactId: 'artifact-destructive' } } } },
      actionIdentifier: 'approve',
    });

    expect(recordDecision).not.toHaveBeenCalled();
  });

  test('default tap (no action identifier) opens the gated review screen instead of deciding anything', async () => {
    const lookup = jest.fn().mockReturnValue(destructiveArtifact);
    (navigationRef.isReady as jest.Mock).mockReturnValue(true);
    addNotificationResponseListener(lookup);

    await fireResponse({
      notification: {
        request: { content: { data: { artifactId: 'artifact-destructive' } } },
      },
      actionIdentifier: 'expo.modules.notifications.actions.DEFAULT',
    });

    expect(recordDecision).not.toHaveBeenCalled();
    expect(navigationRef.navigate).toHaveBeenCalledWith('ArtifactReview', {
      artifactId: 'artifact-destructive',
    });
  });

  test('a response with no artifactId in its data is ignored entirely', async () => {
    const lookup = jest.fn();
    addNotificationResponseListener(lookup);

    await fireResponse({
      notification: { request: { content: { data: {} } } },
      actionIdentifier: 'approve',
    });

    expect(lookup).not.toHaveBeenCalled();
    expect(recordDecision).not.toHaveBeenCalled();
  });

  test('an unknown artifactId from the lookup is ignored rather than throwing', async () => {
    const lookup = jest.fn().mockReturnValue(undefined);
    addNotificationResponseListener(lookup);

    await fireResponse({
      notification: { request: { content: { data: { artifactId: 'ghost' } } } },
      actionIdentifier: 'approve',
    });

    expect(recordDecision).not.toHaveBeenCalled();
  });
});
