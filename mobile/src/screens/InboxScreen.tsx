import { useEffect, useState, useSyncExternalStore } from 'react';
import { NativeStackScreenProps } from '@react-navigation/native-stack';
import { FlatList, Pressable, StyleSheet, Text, TextInput, View } from 'react-native';
import { StatusPill } from '../components/StatusPill';
import { mockSessionThreads } from '../data/mockData';
import { getLocalTasks, subscribeLocalTasks } from '../data/localTaskStore';
import { cacheSessionThreads, getCachedSessionThreads } from '../lib/edgeCache';
import { RootStackParamList } from '../navigation/types';
import { SessionStatus, SessionThread } from '../types/domain';

type Props = NativeStackScreenProps<RootStackParamList, 'Inbox'>;

type StatusFilter = SessionStatus | 'all';

const STATUS_FILTERS: { key: StatusFilter; label: string }[] = [
  { key: 'all', label: 'All' },
  { key: 'running', label: 'Running' },
  { key: 'review', label: 'Review' },
  { key: 'done', label: 'Done' },
];

// Reuses StatusPill's palette (§69.1) so an active filter pill reads as the
// same status color the rows themselves use.
const STATUS_COLOR: Record<SessionStatus, string> = {
  running: '#2563eb',
  review: '#d97706',
  done: '#16a34a',
};

// §69.1's "Inbox/Agent Manager mirror" — the same task-thread list as
// desktop's Inbox (§8, §50.1), read from the same session store once one
// exists. Backed by mock data for now (src/data/mockData.ts), cached
// on-device (§69.5's edge-cached repo context) so reopening the app with no
// connection still shows the last-seen thread list rather than a blank
// screen. Also merges in tasks dictated locally (§69.5's voice-to-task
// capture) via localTaskStore, since there's no backend yet to submit them
// to.
export function InboxScreen({ navigation }: Props) {
  const [threads, setThreads] = useState<SessionThread[]>(mockSessionThreads);
  const [fromCache, setFromCache] = useState(false);
  const [query, setQuery] = useState('');
  const [statusFilter, setStatusFilter] = useState<StatusFilter>('all');
  const localTasks = useSyncExternalStore(subscribeLocalTasks, getLocalTasks);

  useEffect(() => {
    // Mock data always "succeeds" today, so this always takes the live
    // branch and just warms the cache — the fallback branch is exercised
    // once a real, sometimes-unreachable session-store client replaces
    // mockSessionThreads here.
    if (mockSessionThreads.length > 0) {
      setThreads(mockSessionThreads);
      setFromCache(false);
      cacheSessionThreads(mockSessionThreads);
      return;
    }

    getCachedSessionThreads().then((cached) => {
      if (cached) {
        setThreads(cached);
        setFromCache(true);
      }
    });
  }, []);

  const allThreads = [...localTasks, ...threads];
  const normalizedQuery = query.trim().toLowerCase();
  const visibleThreads = allThreads.filter((item) => {
    const matchesQuery =
      normalizedQuery.length === 0 ||
      item.title.toLowerCase().includes(normalizedQuery) ||
      item.workspaceName.toLowerCase().includes(normalizedQuery);
    const matchesStatus = statusFilter === 'all' || item.status === statusFilter;
    return matchesQuery && matchesStatus;
  });

  return (
    <View style={styles.list}>
      {fromCache && (
        <Text style={styles.cacheBanner}>Showing cached threads from your last visit.</Text>
      )}
      <TextInput
        style={styles.searchInput}
        value={query}
        onChangeText={setQuery}
        placeholder="Search threads"
        placeholderTextColor="#9ca3af"
        autoCapitalize="none"
        autoCorrect={false}
      />
      <View style={styles.filterRow}>
        {STATUS_FILTERS.map(({ key, label }) => {
          const active = statusFilter === key;
          const activeColor = key === 'all' ? '#2563eb' : STATUS_COLOR[key];
          return (
            <Pressable
              key={key}
              style={[
                styles.filterPill,
                active && { backgroundColor: activeColor, borderColor: activeColor },
              ]}
              onPress={() => setStatusFilter(key)}
            >
              <Text style={[styles.filterPillText, active && styles.filterPillTextActive]}>
                {label}
              </Text>
            </Pressable>
          );
        })}
      </View>
      <FlatList
        data={visibleThreads}
        keyExtractor={(item) => item.id}
        renderItem={({ item }) => (
          <Pressable
            style={styles.row}
            onPress={() => navigation.navigate('SessionDetail', { sessionId: item.id })}
          >
            <View style={styles.rowHeader}>
              <Text style={styles.title} numberOfLines={1}>
                {item.title}
              </Text>
              <StatusPill status={item.status} />
            </View>
            <Text style={styles.subtitle}>{item.workspaceName}</Text>
            {item.unreadCount > 0 && (
              <View style={styles.unreadBadge}>
                <Text style={styles.unreadText}>{item.unreadCount}</Text>
              </View>
            )}
          </Pressable>
        )}
      />
    </View>
  );
}

const styles = StyleSheet.create({
  list: {
    flex: 1,
    backgroundColor: '#fff',
  },
  cacheBanner: {
    backgroundColor: '#eff6ff',
    color: '#1d4ed8',
    fontSize: 12,
    padding: 10,
  },
  searchInput: {
    marginHorizontal: 16,
    marginTop: 16,
    borderWidth: StyleSheet.hairlineWidth,
    borderColor: '#e5e7eb',
    borderRadius: 8,
    paddingHorizontal: 12,
    paddingVertical: 8,
    fontSize: 14,
  },
  filterRow: {
    flexDirection: 'row',
    gap: 8,
    paddingHorizontal: 16,
    paddingVertical: 12,
  },
  filterPill: {
    paddingHorizontal: 12,
    paddingVertical: 6,
    borderRadius: 999,
    borderWidth: StyleSheet.hairlineWidth,
    borderColor: '#e5e7eb',
  },
  filterPillText: {
    fontSize: 13,
    fontWeight: '600',
    color: '#6b7280',
  },
  filterPillTextActive: {
    color: '#fff',
  },
  row: {
    paddingHorizontal: 16,
    paddingVertical: 14,
    borderBottomWidth: StyleSheet.hairlineWidth,
    borderBottomColor: '#e5e7eb',
  },
  rowHeader: {
    flexDirection: 'row',
    justifyContent: 'space-between',
    alignItems: 'center',
    gap: 8,
  },
  title: {
    fontSize: 16,
    fontWeight: '600',
    flexShrink: 1,
  },
  subtitle: {
    marginTop: 4,
    fontSize: 13,
    color: '#6b7280',
  },
  unreadBadge: {
    position: 'absolute',
    right: 16,
    top: 14,
    backgroundColor: '#ef4444',
    borderRadius: 999,
    minWidth: 18,
    height: 18,
    alignItems: 'center',
    justifyContent: 'center',
    paddingHorizontal: 4,
  },
  unreadText: {
    color: '#fff',
    fontSize: 11,
    fontWeight: '700',
  },
});
