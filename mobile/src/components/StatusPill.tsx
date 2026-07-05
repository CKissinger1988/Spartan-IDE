import { StyleSheet, Text, View } from 'react-native';
import { SessionStatus } from '../types/domain';

const LABEL: Record<SessionStatus, string> = {
  running: 'Running',
  review: 'Review',
  done: 'Done',
};

const COLOR: Record<SessionStatus, string> = {
  running: '#2563eb',
  review: '#d97706',
  done: '#16a34a',
};

export function StatusPill({ status }: { status: SessionStatus }) {
  return (
    <View style={[styles.pill, { backgroundColor: COLOR[status] }]}>
      <Text style={styles.text}>{LABEL[status]}</Text>
    </View>
  );
}

const styles = StyleSheet.create({
  pill: {
    paddingHorizontal: 8,
    paddingVertical: 3,
    borderRadius: 999,
  },
  text: {
    color: '#fff',
    fontSize: 12,
    fontWeight: '600',
  },
});
