import { StyleSheet, Text, View } from 'react-native';
import { SessionStatus } from '../types/domain';
import { STATUS_COLOR } from '../theme';

const LABEL: Record<SessionStatus, string> = {
  running: 'Running',
  review: 'Review',
  done: 'Done',
};

export function StatusPill({ status }: { status: SessionStatus }) {
  const color = STATUS_COLOR[status];
  // Track C: a real status-reactive glow -- the pill's own soft halo takes the
  // status color (running = blue accent, review = amber, done = green), the
  // mobile counterpart of desktop/web's status-reactive glow. shadowColor works
  // on iOS and Android 9+ (elevation carries the Android fallback). Kept subtle
  // so it reads as a HUD accent, never a distraction.
  return (
    <View
      style={[
        styles.pill,
        { backgroundColor: color, shadowColor: color },
      ]}
    >
      <Text style={styles.text}>{LABEL[status]}</Text>
    </View>
  );
}

const styles = StyleSheet.create({
  pill: {
    paddingHorizontal: 8,
    paddingVertical: 3,
    borderRadius: 999,
    // Status-reactive glow (color supplied per-status inline above).
    shadowOffset: { width: 0, height: 0 },
    shadowRadius: 6,
    shadowOpacity: 0.55,
    elevation: 3,
  },
  text: {
    color: '#fff',
    fontSize: 12,
    fontWeight: '600',
  },
});
