import { fireEvent, render, screen, waitFor } from '@testing-library/react-native';
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
    expect(screen.getByText('Approved')).toBeTruthy();
    expect(screen.getByText('Rewrite retry logic')).toBeTruthy();
    expect(screen.getByText('Rejected')).toBeTruthy();
    expect(screen.getByText('Queued — not yet synced')).toBeTruthy();
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
