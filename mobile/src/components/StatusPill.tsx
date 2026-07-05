import { StyleSheet, Text, View } from 'react-native';
import { SessionStatus } from '../types/domain';
import { STATUS_COLOR } from '../theme';

const LABEL: Record<SessionStatus, string> = {
  running: 'Running',
  review: 'Review',
  done: 'Done',
};

export function StatusPill({ status }: { status: SessionStatus }) {
  return (
    <View style={[styles.pill, { backgroundColor: STATUS_COLOR[status] }]}>
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
