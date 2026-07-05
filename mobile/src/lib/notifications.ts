import * as Device from 'expo-device';
import * as Notifications from 'expo-notifications';
import { Platform } from 'react-native';

// §69.1's "Push notifications for review-needed/CI-failure/mention events,
// reusing the existing Notifications settings categories (§42) rather than
// inventing mobile-specific ones." This module only handles the client-side
// half: requesting OS permission and configuring how a notification is
// presented. There is no push token registration against a real backend yet
// — no server exists to send anything to this device. Don't mistake a
// granted permission for a working push pipeline.

Notifications.setNotificationHandler({
  handleNotification: async () => ({
    shouldShowBanner: true,
    shouldShowList: true,
    shouldPlaySound: false,
    shouldSetBadge: false,
  }),
});

export type NotificationPermissionResult =
  | { granted: true }
  | { granted: false; reason: string };

export async function requestNotificationPermission(): Promise<NotificationPermissionResult> {
  if (!Device.isDevice) {
    return {
      granted: false,
      reason: 'Push notifications require a physical device, not a simulator.',
    };
  }

  const existing = await Notifications.getPermissionsAsync();
  let status = existing.status;

  if (status !== 'granted') {
    const requested = await Notifications.requestPermissionsAsync();
    status = requested.status;
  }

  if (status !== 'granted') {
    return { granted: false, reason: 'Permission was not granted.' };
  }

  if (Platform.OS === 'android') {
    await Notifications.setNotificationChannelAsync('review-needed', {
      name: 'Review needed',
      importance: Notifications.AndroidImportance.HIGH,
    });
  }

  return { granted: true };
}
