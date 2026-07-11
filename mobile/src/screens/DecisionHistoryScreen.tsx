import { useCallback, useMemo, useState } from 'react';
import { useFocusEffect } from '@react-navigation/native';
import { FlatList, Pressable, StyleSheet, Text, TextInput, View } from 'react-native';
import { clearDecisionHistory, DecisionHistoryEntry, getDecisionHistory } from '../lib/decisionHistory';
import { useTheme } from '../ThemeContext';
import { ThemeColors } from '../theme';

type DecisionFilter = 'all' | 'approved' | 'rejected';

// A local, persistent log of every Approve/Reject decision made through
// either ArtifactReviewScreen's gated flow or notificationActions.ts's
// direct low-stakes notification-button flow -- both funnel through
// decisionActions.ts's recordDecision, which is what actually appends here.
//
// Search + decision filter (new feature, this pass): a long-lived audit
// log with no way to narrow it stops being useful as it grows past a
// handful of entries -- the same real gap the Inbox screen's own search/
// status filters exist to close, applied here to a different local store.
export function DecisionHistoryScreen() {
  const { colors } = useTheme();
  const styles = useMemo(() => makeStyles(colors), [colors]);
  const decisionFilters = useMemo(
    () => [
      { key: 'all' as const, label: 'All', color: colors.accent },
      { key: 'approved' as const, label: 'Approved', color: colors.green },
      { key: 'rejected' as const, label: 'Rejected', color: colors.red },
    ],
    [colors]
  );
  const [history, setHistory] = useState<DecisionHistoryEntry[]>([]);
  const [query, setQuery] = useState('');
  const [decisionFilter, setDecisionFilter] = useState<DecisionFilter>('all');

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

  const normalizedQuery = query.trim().toLowerCase();
  const visibleHistory = history.filter((entry) => {
    const matchesQuery =
      normalizedQuery.length === 0 || entry.artifactTitle.toLowerCase().includes(normalizedQuery);
    const matchesDecision = decisionFilter === 'all' || entry.decision === decisionFilter;
    return matchesQuery && matchesDecision;
  });
  const emptyMessage =
    history.length === 0
      ? 'No decisions recorded yet.'
      : 'No decisions match your search or filter.';

  return (
    <View style={styles.container}>
      {history.length > 0 && (
        <>
          <TextInput
            style={styles.searchInput}
            value={query}
            onChangeText={setQuery}
            placeholder="Search decisions"
            placeholderTextColor={colors.textDim}
            autoCapitalize="none"
            autoCorrect={false}
          />
          <View style={styles.filterRow}>
            {decisionFilters.map(({ key, label, color }) => {
              const active = decisionFilter === key;
              return (
                <Pressable
                  key={key}
                  style={[
                    styles.filterPill,
                    active && { backgroundColor: color, borderColor: color },
                  ]}
                  onPress={() => setDecisionFilter(key)}
                >
                  <Text style={[styles.filterPillText, active && styles.filterPillTextActive]}>
                    {label}
                  </Text>
                </Pressable>
              );
            })}
          </View>
        </>
      )}
      <FlatList
        data={visibleHistory}
        keyExtractor={(item) => item.id}
        contentContainerStyle={visibleHistory.length === 0 && styles.emptyContainer}
        ListEmptyComponent={<Text style={styles.emptyText}>{emptyMessage}</Text>}
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

function makeStyles(colors: ThemeColors) {
  return StyleSheet.create({
    container: {
      flex: 1,
      backgroundColor: colors.bg,
      padding: 16,
    },
    searchInput: {
      borderWidth: StyleSheet.hairlineWidth,
      borderColor: colors.border,
      borderRadius: 8,
      paddingHorizontal: 12,
      paddingVertical: 8,
      fontSize: 14,
      color: colors.text,
      backgroundColor: colors.s1,
    },
    filterRow: {
      flexDirection: 'row',
      gap: 8,
      paddingVertical: 12,
    },
    filterPill: {
      paddingHorizontal: 12,
      paddingVertical: 6,
      borderRadius: 999,
      borderWidth: StyleSheet.hairlineWidth,
      borderColor: colors.border,
    },
    filterPillText: {
      fontSize: 13,
      fontWeight: '600',
      color: colors.textMid,
    },
    filterPillTextActive: {
      color: colors.text,
    },
    emptyContainer: {
      flexGrow: 1,
      justifyContent: 'center',
      alignItems: 'center',
    },
    emptyText: {
      color: colors.textMid,
      fontSize: 14,
    },
    row: {
      paddingVertical: 12,
      borderBottomWidth: StyleSheet.hairlineWidth,
      borderBottomColor: colors.border,
    },
    rowText: {
      flex: 1,
    },
    title: {
      fontSize: 15,
      fontWeight: '600',
      color: colors.text,
    },
    decision: {
      marginTop: 4,
      fontSize: 13,
      fontWeight: '700',
    },
    approved: {
      color: colors.green,
    },
    rejected: {
      color: colors.red,
    },
    decidedAt: {
      marginTop: 4,
      color: colors.textMid,
      fontSize: 12,
    },
    queuedBadge: {
      marginTop: 4,
      color: colors.textDim,
      fontSize: 12,
      fontStyle: 'italic',
    },
    clearButton: {
      marginTop: 16,
      backgroundColor: colors.s3,
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
      color: colors.textDim,
      fontSize: 12,
    },
  });
}
