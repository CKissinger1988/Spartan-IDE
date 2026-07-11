import { useCallback, useEffect, useState, useSyncExternalStore } from 'react';
import { NativeStackScreenProps } from '@react-navigation/native-stack';
import { FlatList, Pressable, StyleSheet, Text, TextInput, View } from 'react-native';
import { StatusPill } from '../components/StatusPill';
import { mockSessionThreads } from '../data/mockData';
import { getLocalTasks, subscribeLocalTasks } from '../data/localTaskStore';
import { cacheSessionThreads, getCachedSessionThreads } from '../lib/edgeCache';
import { RootStackParamList } from '../navigation/types';
import { C, STATUS_COLOR } from '../theme';
import { SessionStatus, SessionThread } from '../types/domain';

type Props = NativeStackScreenProps<RootStackParamList, 'Inbox'>;

type StatusFilter = SessionStatus | 'all';

const STATUS_FILTERS: { key: StatusFilter; label: string }[] = [
  { key: 'all', label: 'All' },
  { key: 'running', label: 'Running' },
  { key: 'review', label: 'Review' },
  { key: 'done', label: 'Done' },
];

const ALL_WORKSPACES = 'All Workspaces';

// §69.1's "Inbox/Agent Manager mirror" — the same task-thread list as
// desktop's Inbox (§8, §50.1), read from the same session store once one
// exists. Backed by mock data for now (src/data/mockData.ts), cached
// on-device (§69.5's edge-cached repo context) so reopening the app with no
// connection still shows the last-seen thread list rather than a blank
// screen. Also merges in tasks dictated locally (§69.5's voice-to-task
// capture) via localTaskStore, since there's no backend yet to submit them
// to.
//
// Workspace filter and pull-to-refresh (new feature, this pass): threads
// already carry a real `workspaceName` (desktop's own multi-workspace
// concept, §8), but the Inbox never let a reviewer narrow to just one --
// with more than a couple of active workspaces this list only gets
// noisier. The filter's own option list is derived from whatever
// workspace names are actually present (mock + local tasks combined),
// never hardcoded, so it stays correct as threads come and go. Pull-to-
// refresh re-runs the exact same live/cache load the initial mount does
// -- honest today (mock data can't meaningfully "refresh"), and becomes
// the real re-fetch path for free once a live session-store client
// replaces `mockSessionThreads`.
export function InboxScreen({ navigation }: Props) {
  const [threads, setThreads] = useState<SessionThread[]>(mockSessionThreads);
  const [fromCache, setFromCache] = useState(false);
  const [query, setQuery] = useState('');
  const [statusFilter, setStatusFilter] = useState<StatusFilter>('all');
  const [workspaceFilter, setWorkspaceFilter] = useState<string>(ALL_WORKSPACES);
  const [refreshing, setRefreshing] = useState(false);
  const localTasks = useSyncExternalStore(subscribeLocalTasks, getLocalTasks);

  const load = useCallback(() => {
    // Mock data always "succeeds" today, so this always takes the live
    // branch and just warms the cache — the fallback branch is exercised
    // once a real, sometimes-unreachable session-store client replaces
    // mockSessionThreads here.
    if (mockSessionThreads.length > 0) {
      setThreads(mockSessionThreads);
      setFromCache(false);
      cacheSessionThreads(mockSessionThreads);
      return Promise.resolve();
    }

    return getCachedSessionThreads().then((cached) => {
      if (cached) {
        setThreads(cached);
        setFromCache(true);
      }
    });
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  const onRefresh = useCallback(() => {
    setRefreshing(true);
    load().finally(() => setRefreshing(false));
  }, [load]);

  const allThreads = [...localTasks, ...threads];
  const workspaceNames = Array.from(new Set(allThreads.map((item) => item.workspaceName))).sort();
  const normalizedQuery = query.trim().toLowerCase();
  const visibleThreads = allThreads.filter((item) => {
    const matchesQuery =
      normalizedQuery.length === 0 ||
      item.title.toLowerCase().includes(normalizedQuery) ||
      item.workspaceName.toLowerCase().includes(normalizedQuery);
    const matchesStatus = statusFilter === 'all' || item.status === statusFilter;
    const matchesWorkspace =
      workspaceFilter === ALL_WORKSPACES || item.workspaceName === workspaceFilter;
    return matchesQuery && matchesStatus && matchesWorkspace;
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
        placeholderTextColor={C.textDim}
        autoCapitalize="none"
        autoCorrect={false}
      />
      <View style={styles.filterRow}>
        {STATUS_FILTERS.map(({ key, label }) => {
          const active = statusFilter === key;
          const activeColor = key === 'all' ? C.accent : STATUS_COLOR[key];
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
      {workspaceNames.length > 1 && (
        <View style={styles.filterRow}>
          {[ALL_WORKSPACES, ...workspaceNames].map((name) => {
            const active = workspaceFilter === name;
            return (
              <Pressable
                key={name}
                style={[
                  styles.filterPill,
                  active && { backgroundColor: C.accent, borderColor: C.accent },
                ]}
                onPress={() => setWorkspaceFilter(name)}
              >
                <Text style={[styles.filterPillText, active && styles.filterPillTextActive]}>
                  {name}
                </Text>
              </Pressable>
            );
          })}
        </View>
      )}
      <FlatList
        testID="inbox-thread-list"
        data={visibleThreads}
        keyExtractor={(item) => item.id}
        refreshing={refreshing}
        onRefresh={onRefresh}
        renderItem={({ item }) => (
          <Pressable
            style={styles.row}
            onPress={() => navigation.navigate('SessionDetail', { sessionId: item.id })}
          >
            <View style={styles.rowHeader}>
              <Text style={styles.title} numberOfLines={1}>
                {item.title}
              </Text>
              <View style={styles.rowHeaderRight}>
                {item.unreadCount > 0 && (
                  <View style={styles.unreadBadge} testID={`unread-badge-${item.id}`}>
                    <Text style={styles.unreadText}>{item.unreadCount}</Text>
                  </View>
                )}
                <StatusPill status={item.status} />
              </View>
            </View>
            <Text style={styles.subtitle}>{item.workspaceName}</Text>
          </Pressable>
        )}
      />
    </View>
  );
}

const styles = StyleSheet.create({
  list: {
    flex: 1,
    backgroundColor: C.bg,
  },
  cacheBanner: {
    backgroundColor: C.accentBg,
    color: C.accent,
    fontSize: 12,
    padding: 10,
  },
  searchInput: {
    marginHorizontal: 16,
    marginTop: 16,
    borderWidth: StyleSheet.hairlineWidth,
    borderColor: C.border,
    borderRadius: 8,
    paddingHorizontal: 12,
    paddingVertical: 8,
    fontSize: 14,
    color: C.text,
    backgroundColor: C.s1,
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
    borderColor: C.border,
  },
  filterPillText: {
    fontSize: 13,
    fontWeight: '600',
    color: C.textMid,
  },
  filterPillTextActive: {
    color: C.text,
  },
  row: {
    paddingHorizontal: 16,
    paddingVertical: 14,
    borderBottomWidth: StyleSheet.hairlineWidth,
    borderBottomColor: C.border,
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
    color: C.text,
  },
  subtitle: {
    marginTop: 4,
    fontSize: 13,
    color: C.textMid,
  },
  rowHeaderRight: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: 6,
    flexShrink: 0,
  },
  unreadBadge: {
    backgroundColor: C.red,
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
