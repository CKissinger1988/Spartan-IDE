import { act, fireEvent, render, screen, waitFor } from '@testing-library/react-native';
import { DecisionHistoryScreen } from '../DecisionHistoryScreen';
import { clearDecisionHistory, DecisionHistoryEntry, getDecisionHistory } from '../../lib/decisionHistory';

jest.mock('../../lib/decisionHistory', () => ({
  getDecisionHistory: jest.fn(),
  clearDecisionHistory: jest.fn(),
}));

jest.mock('@react-navigation/native', () => ({
  ...jest.requireActual('@react-navigation/native'),
  useFocusEffect: (effect: () => void) => require('react').useEffect(effect, [effect]),
}));

const mockGetDecisionHistory = getDecisionHistory as jest.Mock;
const mockClearDecisionHistory = clearDecisionHistory as jest.Mock;

const entries: DecisionHistoryEntry[] = [
  {
    id: 'decision-1',
    artifactId: 'artifact-1',
    artifactTitle: 'Refactor auth middleware',
    decision: 'approved',
    decidedAt: '2026-01-01T00:00:00Z',
    queued: false,
  },
  {
    id: 'decision-2',
    artifactId: 'artifact-2',
    artifactTitle: 'Rewrite retry logic',
    decision: 'rejected',
    decidedAt: '2026-01-02T00:00:00Z',
    queued: true,
  },
];

describe('DecisionHistoryScreen', () => {
  beforeEach(() => {
    mockGetDecisionHistory.mockReset();
    mockClearDecisionHistory.mockReset();
  });

  test('renders entries returned by getDecisionHistory, including a queued indicator', async () => {
    mockGetDecisionHistory.mockResolvedValue(entries);

    await render(<DecisionHistoryScreen />);

    expect(await screen.findByText('Refactor auth middleware')).toBeTruthy();
    // "Approved"/"Rejected" also label the decision-filter pills now, so a
    // row's own decision label is one of possibly several matches.
    expect(screen.getAllByText('Approved').length).toBeGreaterThan(0);
    expect(screen.getByText('Rewrite retry logic')).toBeTruthy();
    expect(screen.getAllByText('Rejected').length).toBeGreaterThan(0);
    expect(screen.getByText('Queued — not yet synced')).toBeTruthy();
  });

  test('search input filters entries by artifact title substring', async () => {
    mockGetDecisionHistory.mockResolvedValue(entries);
    await render(<DecisionHistoryScreen />);
    expect(await screen.findByText('Refactor auth middleware')).toBeTruthy();

    await act(async () => {
      fireEvent.changeText(screen.getByPlaceholderText('Search decisions'), 'retry');
    });

    expect(screen.getByText('Rewrite retry logic')).toBeTruthy();
    expect(screen.queryByText('Refactor auth middleware')).toBeNull();
  });

  test('decision filter pill narrows the list to that decision only', async () => {
    mockGetDecisionHistory.mockResolvedValue(entries);
    await render(<DecisionHistoryScreen />);
    expect(await screen.findByText('Refactor auth middleware')).toBeTruthy();

    await act(async () => {
      fireEvent.press(screen.getAllByText('Rejected')[0]);
    });

    expect(screen.getByText('Rewrite retry logic')).toBeTruthy();
    expect(screen.queryByText('Refactor auth middleware')).toBeNull();
  });

  test('a non-matching search shows a distinct "no match" message, not the empty-history message', async () => {
    mockGetDecisionHistory.mockResolvedValue(entries);
    await render(<DecisionHistoryScreen />);
    expect(await screen.findByText('Refactor auth middleware')).toBeTruthy();

    await act(async () => {
      fireEvent.changeText(screen.getByPlaceholderText('Search decisions'), 'nonexistent');
    });

    expect(screen.getByText('No decisions match your search or filter.')).toBeTruthy();
    expect(screen.queryByText('No decisions recorded yet.')).toBeNull();
  });

  test('the search box and filter row do not render when history is genuinely empty', async () => {
    mockGetDecisionHistory.mockResolvedValue([]);
    await render(<DecisionHistoryScreen />);
    expect(await screen.findByText('No decisions recorded yet.')).toBeTruthy();
    expect(screen.queryByPlaceholderText('Search decisions')).toBeNull();
  });

  test('shows an empty-state message when getDecisionHistory resolves []', async () => {
    mockGetDecisionHistory.mockResolvedValue([]);

    await render(<DecisionHistoryScreen />);

    expect(await screen.findByText('No decisions recorded yet.')).toBeTruthy();
  });

  test('Clear history calls clearDecisionHistory and the list reflects empty', async () => {
    mockGetDecisionHistory.mockResolvedValueOnce(entries);
    mockClearDecisionHistory.mockResolvedValue(undefined);

    await render(<DecisionHistoryScreen />);

    expect(await screen.findByText('Refactor auth middleware')).toBeTruthy();

    mockGetDecisionHistory.mockResolvedValueOnce([]);
    await fireEvent.press(screen.getByText('Clear history'));

    await waitFor(() => expect(mockClearDecisionHistory).toHaveBeenCalledTimes(1));
    expect(await screen.findByText('No decisions recorded yet.')).toBeTruthy();
  });
});
