import { useCallback, useMemo, useState } from 'react';
import { NativeStackScreenProps } from '@react-navigation/native-stack';
import { Alert, Pressable, StyleSheet, Switch, Text, View } from 'react-native';
import { useFocusEffect } from '@react-navigation/native';
import { mockArtifacts } from '../data/mockData';
import { getConnectivitySnapshot } from '../lib/network';
import { requestNotificationPermission } from '../lib/notifications';
import { schedulePreviewNotification } from '../lib/notificationActions';
import { clearQueue, getQueuedDecisions, replayQueue } from '../lib/offlineQueue';
import { RootStackParamList } from '../navigation/types';
import { useTheme } from '../ThemeContext';
import { MONO_FONT_FAMILY, ThemeColors } from '../theme';
import { QueuedDecision } from '../types/queue';

type Props = NativeStackScreenProps<RootStackParamList, 'Settings'>;

// §69.1's push-notification settings, reusing the desktop's existing
// Notifications categories (§42) conceptually rather than inventing
// mobile-only ones — this screen is the client-side permission toggle only;
// there's no backend yet to actually deliver a push to this toggle.
export function SettingsScreen({ navigation }: Props) {
  const { mode, colors, setMode } = useTheme();
  const styles = useMemo(() => makeStyles(colors), [colors]);
  const [notificationsEnabled, setNotificationsEnabled] = useState(false);
  const [requesting, setRequesting] = useState(false);
  const [queue, setQueue] = useState<QueuedDecision[]>([]);
  const [connectivity, setConnectivity] = useState<{ isConnected: boolean; isWifi: boolean } | null>(
    null
  );

  const refresh = useCallback(() => {
    getQueuedDecisions().then(setQueue);
    getConnectivitySnapshot().then(setConnectivity);
  }, []);

  // Re-check connectivity and the queue every time this screen is opened —
  // §69.5's offline-first review queue only means anything if its status is
  // visible, not just something happening silently in AsyncStorage.
  useFocusEffect(
    useCallback(() => {
      refresh();
    }, [refresh])
  );

  const handleToggle = async (next: boolean) => {
    if (!next) {
      setNotificationsEnabled(false);
      return;
    }

    setRequesting(true);
    const result = await requestNotificationPermission();
    setRequesting(false);

    if (result.granted) {
      setNotificationsEnabled(true);
    } else {
      setNotificationsEnabled(false);
      Alert.alert('Notifications unavailable', result.reason);
    }
  };

  const handleReplay = async () => {
    const result = await replayQueue();
    Alert.alert('Sync attempted', `${result.attempted} queued. ${result.note}`);
  };

  const handleClear = async () => {
    await clearQueue();
    refresh();
  };

  const handleSendPreview = async (lowStakes: boolean) => {
    const artifact = mockArtifacts.find((a) => a.lowStakes === lowStakes);
    if (!artifact) return;
    await schedulePreviewNotification(artifact);
  };

  return (
    <View style={styles.container}>
      <View style={styles.row}>
        <View style={styles.rowText}>
          <Text style={styles.label}>Review-needed notifications</Text>
          <Text style={styles.description}>
            Push notifications when a diff, plan, or CI failure needs your review.
          </Text>
        </View>
        <Switch
          value={notificationsEnabled}
          onValueChange={handleToggle}
          disabled={requesting}
          trackColor={{ false: colors.s3, true: colors.accent }}
          thumbColor={colors.text}
        />
      </View>
      <Text style={styles.note}>
        No push backend exists yet — this only requests OS permission and configures how a
        notification would be presented. Nothing can actually be sent to this device yet.
      </Text>

      <View style={styles.section}>
        <Text style={styles.sectionHeader}>Appearance</Text>
        <View style={styles.row}>
          <View style={styles.rowText}>
            <Text style={styles.label}>Theme</Text>
            <Text style={styles.description}>
              Applies live, everywhere in the app -- no restart needed.
            </Text>
          </View>
          <View style={styles.queueActions}>
            <Pressable
              style={[styles.themeButton, mode === 'dark' && styles.themeButtonActive]}
              onPress={() => setMode('dark')}
            >
              <Text style={[styles.themeButtonText, mode === 'dark' && styles.themeButtonTextActive]}>
                Dark
              </Text>
            </Pressable>
            <Pressable
              style={[styles.themeButton, mode === 'light' && styles.themeButtonActiveGold]}
              onPress={() => setMode('light')}
            >
              <Text style={[styles.themeButtonText, mode === 'light' && styles.themeButtonTextActive]}>
                Light
              </Text>
            </Pressable>
          </View>
        </View>
      </View>

      <View style={styles.section}>
        <Text style={styles.sectionHeader}>Connectivity</Text>
        <Text style={styles.description}>
          {connectivity === null
            ? 'Checking…'
            : connectivity.isConnected
              ? connectivity.isWifi
                ? 'Connected — Wi-Fi'
                : 'Connected — cellular'
              : 'Offline'}
        </Text>
      </View>

      <View style={styles.section}>
        <Text style={styles.sectionHeader}>Pending sync queue ({queue.length})</Text>
        {queue.length === 0 ? (
          <Text style={styles.description}>No decisions queued.</Text>
        ) : (
          queue.map((item) => (
            <Text key={item.id} style={styles.queueRow}>
              {item.decision} · {item.artifactId} · queued {item.queuedAt}
            </Text>
          ))
        )}
        <View style={styles.queueActions}>
          <Pressable style={styles.queueButton} onPress={handleReplay}>
            <Text style={styles.queueButtonText}>Attempt sync</Text>
          </Pressable>
          <Pressable style={[styles.queueButton, styles.queueButtonSecondary]} onPress={handleClear}>
            <Text style={styles.queueButtonText}>Clear queue</Text>
          </Pressable>
        </View>
        <Text style={styles.note}>
          "Attempt sync" is a stub — there is no backend yet to actually send a queued decision
          to. It only reports what's currently queued.
        </Text>
      </View>

      <View style={styles.section}>
        <Text style={styles.sectionHeader}>Decision history</Text>
        <Text style={styles.description}>
          See every Approve/Reject decision recorded on this device, whether it applied live or
          was queued offline.
        </Text>
        <Pressable
          style={[styles.queueButton, styles.historyButton]}
          onPress={() => navigation.navigate('DecisionHistory')}
        >
          <Text style={styles.queueButtonText}>View decision history</Text>
        </Pressable>
      </View>

      <View style={styles.section}>
        <Text style={styles.sectionHeader}>Notification-surface actions</Text>
        <Text style={styles.description}>
          Send yourself a local preview notification to try the Approve/Reject action buttons
          (low-stakes) or the open-to-review path (destructive class) — §69.5.
        </Text>
        <View style={styles.queueActions}>
          <Pressable style={styles.queueButton} onPress={() => handleSendPreview(true)}>
            <Text style={styles.queueButtonText}>Preview: low-stakes</Text>
          </Pressable>
          <Pressable style={[styles.queueButton, styles.queueButtonSecondary]} onPress={() => handleSendPreview(false)}>
            <Text style={styles.queueButtonText}>Preview: destructive</Text>
          </Pressable>
        </View>
      </View>
    </View>
  );
}

// Real §75.93 reactive stylesheet -- a function of the real, currently
// selected theme's colors instead of a fixed module-scope `StyleSheet.
// create` bound to the dark palette, so this screen re-renders correctly
// the instant the theme changes (`useMemo(() => makeStyles(colors),
// [colors])` above).
function makeStyles(colors: ThemeColors) {
  return StyleSheet.create({
    container: {
      flex: 1,
      backgroundColor: colors.bg,
      padding: 16,
    },
    row: {
      flexDirection: 'row',
      alignItems: 'center',
      justifyContent: 'space-between',
      gap: 12,
    },
    rowText: {
      flex: 1,
    },
    label: {
      fontSize: 16,
      fontWeight: '600',
      color: colors.text,
    },
    description: {
      marginTop: 4,
      color: colors.textMid,
      fontSize: 13,
    },
    note: {
      marginTop: 12,
      color: colors.textDim,
      fontSize: 12,
    },
    section: {
      marginTop: 28,
    },
    sectionHeader: {
      fontSize: 15,
      fontWeight: '700',
      color: colors.text,
    },
    queueRow: {
      marginTop: 8,
      fontFamily: MONO_FONT_FAMILY,
      fontSize: 12,
      color: colors.textMid,
    },
    queueActions: {
      flexDirection: 'row',
      gap: 10,
      marginTop: 14,
    },
    queueButton: {
      backgroundColor: colors.accent,
      borderRadius: 8,
      paddingVertical: 10,
      paddingHorizontal: 14,
    },
    queueButtonSecondary: {
      backgroundColor: colors.s3,
    },
    historyButton: {
      marginTop: 14,
      alignSelf: 'flex-start',
    },
    queueButtonText: {
      color: '#fff',
      fontWeight: '700',
      fontSize: 13,
    },
    themeButton: {
      borderRadius: 8,
      paddingVertical: 8,
      paddingHorizontal: 14,
      borderWidth: 1,
      borderColor: colors.border,
    },
    themeButtonActive: {
      backgroundColor: colors.accent,
      borderColor: colors.accent,
    },
    // Real §75.95 rebrand: the Light-mode pill uses the real gold secondary
    // accent rather than the primary blue `themeButtonActive` above -- a
    // real, deliberate, visible "both brand colors together" moment (Dark
    // -> blue, Light -> gold) rather than one accent used for both toggle
    // states, the only place in this app that shows `colors.gold` today.
    themeButtonActiveGold: {
      backgroundColor: colors.gold,
      borderColor: colors.gold,
    },
    themeButtonText: {
      color: colors.text,
      fontWeight: '600',
      fontSize: 13,
    },
    themeButtonTextActive: {
      // Always white, regardless of theme -- both `themeButtonActive` and
      // `themeButtonActiveGold`'s own backgrounds are real, saturated
      // accent colors in both themes, and plain `colors.text` (near-black
      // in the light theme) would be unreadable against either, the same
      // real reasoning `queueButtonText` above already applies to every
      // other accent-colored button.
      color: '#fff',
    },
  });
}
