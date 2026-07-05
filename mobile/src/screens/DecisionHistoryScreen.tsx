import { useCallback, useState } from 'react';
import { useFocusEffect } from '@react-navigation/native';
import { FlatList, Pressable, StyleSheet, Text, View } from 'react-native';
import { clearDecisionHistory, DecisionHistoryEntry, getDecisionHistory } from '../lib/decisionHistory';

// A local, persistent log of every Approve/Reject decision made through
// either ArtifactReviewScreen's gated flow or notificationActions.ts's
// direct low-stakes notification-button flow -- both funnel through
// decisionActions.ts's recordDecision, which is what actually appends here.
export function DecisionHistoryScreen() {
  const [history, setHistory] = useState<DecisionHistoryEntry[]>([]);

  const refresh = useCallback(() => {
    getDecisionHistory().then(setHistory);
  }, []);

  useFocusEffect(
    useCallback(() => {
      refresh();
    }, [refresh])
  );

  const handleClear = async () => {
    await clearDecisionHistory();
    refresh();
  };

  return (
    <View style={styles.container}>
      <FlatList
        data={history}
        keyExtractor={(item) => item.id}
        contentContainerStyle={history.length === 0 && styles.emptyContainer}
        ListEmptyComponent={<Text style={styles.emptyText}>No decisions recorded yet.</Text>}
        renderItem={({ item }) => (
          <View style={styles.row}>
            <View style={styles.rowText}>
              <Text style={styles.title}>{item.artifactTitle}</Text>
              <Text
                style={[
                  styles.decision,
                  item.decision === 'approved' ? styles.approved : styles.rejected,
                ]}
              >
                {item.decision === 'approved' ? 'Approved' : 'Rejected'}
              </Text>
              <Text style={styles.decidedAt}>{item.decidedAt}</Text>
              {item.queued && <Text style={styles.queuedBadge}>Queued — not yet synced</Text>}
            </View>
          </View>
        )}
      />
      <Pressable style={styles.clearButton} onPress={handleClear}>
        <Text style={styles.clearButtonText}>Clear history</Text>
      </Pressable>
      <Text style={styles.note}>
        Local only — this is a log kept on this device, not a synced audit trail. No backend
        exists yet to reconcile it against.
      </Text>
    </View>
  );
}

const styles = StyleSheet.create({
  container: {
    flex: 1,
    backgroundColor: '#fff',
    padding: 16,
  },
  emptyContainer: {
    flexGrow: 1,
    justifyContent: 'center',
    alignItems: 'center',
  },
  emptyText: {
    color: '#6b7280',
    fontSize: 14,
  },
  row: {
    paddingVertical: 12,
    borderBottomWidth: StyleSheet.hairlineWidth,
    borderBottomColor: '#e5e7eb',
  },
  rowText: {
    flex: 1,
  },
  title: {
    fontSize: 15,
    fontWeight: '600',
  },
  decision: {
    marginTop: 4,
    fontSize: 13,
    fontWeight: '700',
  },
  approved: {
    color: '#16a34a',
  },
  rejected: {
    color: '#ef4444',
  },
  decidedAt: {
    marginTop: 4,
    color: '#6b7280',
    fontSize: 12,
  },
  queuedBadge: {
    marginTop: 4,
    color: '#9ca3af',
    fontSize: 12,
    fontStyle: 'italic',
  },
  clearButton: {
    marginTop: 16,
    backgroundColor: '#6b7280',
    borderRadius: 8,
    paddingVertical: 12,
    alignItems: 'center',
  },
  clearButtonText: {
    color: '#fff',
    fontWeight: '700',
  },
  note: {
    marginTop: 12,
    color: '#9ca3af',
    fontSize: 12,
  },
});
