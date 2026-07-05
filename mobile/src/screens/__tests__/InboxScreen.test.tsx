import React from 'react';
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react-native';
import { InboxScreen } from '../InboxScreen';
import { mockSessionThreads } from '../../data/mockData';
import { getLocalTasks, subscribeLocalTasks } from '../../data/localTaskStore';
import { cacheSessionThreads, getCachedSessionThreads } from '../../lib/edgeCache';

jest.mock('../../data/localTaskStore', () => {
  const noop = () => {};
  // useSyncExternalStore compares snapshots with Object.is -- a fresh array
  // literal on every call reads as "changed" and loops forever, so the
  // default mock must return the same reference every time.
  const emptyTasks: unknown[] = [];
  return {
    subscribeLocalTasks: jest.fn(() => noop),
    getLocalTasks: jest.fn(() => emptyTasks),
  };
});

jest.mock('../../lib/edgeCache', () => ({
  cacheSessionThreads: jest.fn(),
  getCachedSessionThreads: jest.fn(),
}));

afterEach(async () => {
  await cleanup();
});

async function renderScreen() {
  const navigation = { navigate: jest.fn(), goBack: jest.fn() };
  const route = { params: undefined };
  await render(<InboxScreen navigation={navigation as any} route={route as any} />);
  return { navigation };
}

// The filter row renders above the thread list, so its pill label (e.g.
// "Done") is always the first match -- a status pill on a matching row can
// carry the same text.
async function pressFilterPill(label: string) {
  await act(async () => {
    fireEvent.press(screen.getAllByText(label)[0]);
  });
}

async function typeSearch(query: string) {
  await act(async () => {
    fireEvent.changeText(screen.getByPlaceholderText('Search threads'), query);
  });
}

describe('InboxScreen', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    (subscribeLocalTasks as jest.Mock).mockReturnValue(() => {});
    (getLocalTasks as jest.Mock).mockReturnValue([]);
    (cacheSessionThreads as jest.Mock).mockResolvedValue(undefined);
    (getCachedSessionThreads as jest.Mock).mockResolvedValue(null);
  });

  test('renders every mock session thread with its title and workspace', async () => {
    await renderScreen();
    const workspaceNames = new Set(mockSessionThreads.map((thread) => thread.workspaceName));
    for (const thread of mockSessionThreads) {
      expect(screen.getByText(thread.title)).toBeTruthy();
    }
    for (const workspaceName of workspaceNames) {
      expect(screen.getAllByText(workspaceName).length).toBeGreaterThan(0);
    }
  });

  test('renders each thread status as its pill label', async () => {
    await renderScreen();
    expect(screen.getAllByText('Review').length).toBeGreaterThan(0);
    expect(screen.getAllByText('Running').length).toBeGreaterThan(0);
    expect(screen.getAllByText('Done').length).toBeGreaterThan(0);
  });

  test('tapping a row navigates to SessionDetail with that thread id', async () => {
    const { navigation } = await renderScreen();
    const target = mockSessionThreads[0];
    await act(async () => {
      fireEvent.press(screen.getByText(target.title));
    });
    expect(navigation.navigate).toHaveBeenCalledWith('SessionDetail', { sessionId: target.id });
  });

  test('search input filters rows by title substring', async () => {
    await renderScreen();
    await typeSearch('checkout');
    expect(screen.getByText('Fix checkout race condition')).toBeTruthy();
    expect(screen.queryByText('Add retry backoff to webhook sender')).toBeNull();
    expect(screen.queryByText('Migrate settings panel to new theme tokens')).toBeNull();
  });

  test('search input filters rows by workspace substring', async () => {
    await renderScreen();
    await typeSearch('spartan-ide-desktop');
    expect(screen.getByText('Migrate settings panel to new theme tokens')).toBeTruthy();
    expect(screen.queryByText('Fix checkout race condition')).toBeNull();
    expect(screen.queryByText('Add retry backoff to webhook sender')).toBeNull();
  });

  test('status filter pill narrows the list to the selected status', async () => {
    await renderScreen();
    await pressFilterPill('Done');
    expect(screen.getByText('Migrate settings panel to new theme tokens')).toBeTruthy();
    expect(screen.queryByText('Fix checkout race condition')).toBeNull();
    expect(screen.queryByText('Add retry backoff to webhook sender')).toBeNull();
  });

  test('search and status filter combine', async () => {
    await renderScreen();
    await pressFilterPill('Running');
    await typeSearch('webhook');
    expect(screen.getByText('Add retry backoff to webhook sender')).toBeTruthy();
    expect(screen.queryByText('Fix checkout race condition')).toBeNull();
    expect(screen.queryByText('Migrate settings panel to new theme tokens')).toBeNull();
  });

  test('combining a non-matching search with a status filter yields no rows', async () => {
    await renderScreen();
    await pressFilterPill('Done');
    await typeSearch('webhook');
    expect(screen.queryByText('Fix checkout race condition')).toBeNull();
    expect(screen.queryByText('Add retry backoff to webhook sender')).toBeNull();
    expect(screen.queryByText('Migrate settings panel to new theme tokens')).toBeNull();
  });
});
