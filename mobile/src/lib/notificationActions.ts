import * as Notifications from 'expo-notifications';
import { Platform } from 'react-native';
import { navigationRef } from '../navigation/navigationRef';
import { Artifact } from '../types/domain';
import { recordDecision } from './decisionActions';

// §69.5's "Notification-surface actions (v1)": Approve/Reject directly from
// the push notification for low-stakes artifacts; anything in the
// destructive class always requires opening the app and passing the
// biometric gate — the lock screen never approves a `git push --force`.
//
// No push backend exists yet (see the README), so nothing here is reachable
// from a real remote push today. This wires up the category/action/listener
// plumbing so it's a real, testable client-side feature on its own, and a
// one-line change (pointing a real push payload at these category
// identifiers) once a backend exists to send one.

const LOW_STAKES_CATEGORY = 'review-actionable-low-stakes';
const DESTRUCTIVE_CATEGORY = 'review-actionable-destructive';

export async function registerNotificationCategories(): Promise<void> {
  // expo-notifications doesn't implement notification categories on
  // react-native-web — setNotificationCategoryAsync throws there. Nothing
  // web-specific needs them (no push surface exists on web either), so skip
  // rather than let an unhandled rejection hit every web load.
  if (Platform.OS === 'web') return;

  await Notifications.setNotificationCategoryAsync(LOW_STAKES_CATEGORY, [
    { identifier: 'approve', buttonTitle: 'Approve' },
    { identifier: 'reject', buttonTitle: 'Reject', options: { isDestructive: true } },
  ]);

  // Destructive-class artifacts get no direct action buttons at all —
  // tapping the notification only opens the app to the gated review screen.
  await Notifications.setNotificationCategoryAsync(DESTRUCTIVE_CATEGORY, []);
}

export function categoryForArtifact(artifact: Artifact): string {
  return artifact.lowStakes ? LOW_STAKES_CATEGORY : DESTRUCTIVE_CATEGORY;
}

// Local-only stand-in for what would otherwise be a remote push payload —
// there is no server to actually deliver one, so this schedules an
// immediate local notification so the action-button/listener path can be
// exercised for real.
export async function schedulePreviewNotification(artifact: Artifact): Promise<void> {
  await Notifications.scheduleNotificationAsync({
    content: {
      title: artifact.lowStakes ? 'Review needed (low-stakes)' : 'Review needed',
      body: artifact.title,
      data: { artifactId: artifact.id },
      categoryIdentifier: categoryForArtifact(artifact),
    },
    trigger: null,
  });
}

export function addNotificationResponseListener(
  artifactLookup: (artifactId: string) => Artifact | undefined
) {
  return Notifications.addNotificationResponseReceivedListener(async (response) => {
    const artifactId = response.notification.request.content.data?.artifactId as
      | string
      | undefined;
    if (!artifactId) return;

    const actionIdentifier = response.actionIdentifier;

    if (actionIdentifier === 'approve' || actionIdentifier === 'reject') {
      const artifact = artifactLookup(artifactId);
      if (!artifact) return;

      // Only reachable for low-stakes artifacts in practice, since the
      // destructive category (above) never registers these action
      // identifiers — but the check is kept explicit here too rather than
      // trusted implicitly from category wiring alone.
      if (!artifact.lowStakes) return;

      await recordDecision(artifact, actionIdentifier === 'approve' ? 'approved' : 'rejected', {
        requireBiometric: false,
      });
      return;
    }

    // Default tap (or the destructive category's only path): open the app
    // to the gated review screen instead of deciding anything here.
    if (navigationRef.isReady()) {
      navigationRef.navigate('ArtifactReview', { artifactId });
    }
  });
}
