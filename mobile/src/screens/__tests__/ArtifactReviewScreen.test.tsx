import { Alert } from 'react-native';
import { fireEvent, render, waitFor } from '@testing-library/react-native';
import { ArtifactReviewScreen } from '../ArtifactReviewScreen';
import { cacheArtifact, getCachedArtifact } from '../../lib/edgeCache';
import { recordDecision } from '../../lib/decisionActions';
import { localModelClient } from '../../lib/localModel';
import { mockArtifacts } from '../../data/mockData';

jest.mock('../../lib/edgeCache', () => ({
  cacheArtifact: jest.fn(),
  getCachedArtifact: jest.fn(),
}));
jest.mock('../../lib/decisionActions', () => ({
  recordDecision: jest.fn(),
}));
jest.mock('../../lib/localModel', () => ({
  localModelClient: {
    isAvailable: jest.fn(),
    ask: jest.fn(),
  },
}));

const artifact = mockArtifacts.find((a) => a.id === 'art-1')!;
const route = { params: { artifactId: 'art-1' } } as any;
const navigation = { navigate: jest.fn(), goBack: jest.fn() } as any;

describe('ArtifactReviewScreen', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    (cacheArtifact as jest.Mock).mockResolvedValue(undefined);
    (getCachedArtifact as jest.Mock).mockResolvedValue(null);
    (recordDecision as jest.Mock).mockResolvedValue({ status: 'recorded', queued: false });
    (localModelClient.isAvailable as jest.Mock).mockResolvedValue(false);
  });

  test('renders artifact title, summary, and diff content', async () => {
    const { findByText, getByText } = await render(
      <ArtifactReviewScreen route={route} navigation={navigation} />
    );

    expect(await findByText(artifact.title)).toBeTruthy();
    expect(getByText(artifact.summary)).toBeTruthy();
    expect(getByText(artifact.files[0].path)).toBeTruthy();
  });

  test('diff lines render their real text content -- added, removed, and context lines', async () => {
    const { findByText, getByText } = await render(
      <ArtifactReviewScreen route={route} navigation={navigation} />
    );
    await findByText(artifact.title);

    expect(getByText('+  const latestStock = await db.stock.read(order.sku);')).toBeTruthy();
    expect(getByText('-  await db.orders.insert(order);')).toBeTruthy();
    expect(
      getByText('@@ -22,7 +22,10 @@ export async function commitOrder(order: PendingOrder) {')
    ).toBeTruthy();
    expect(getByText('+  await db.orders.insert(order);')).toBeTruthy();
  });

  test('tapping Approve calls recordDecision with the artifact and "approved"', async () => {
    const { findByText, getByText } = await render(
      <ArtifactReviewScreen route={route} navigation={navigation} />
    );
    await findByText(artifact.title);

    await fireEvent.press(getByText('Approve'));

    await waitFor(() => expect(recordDecision).toHaveBeenCalledTimes(1));
    expect(recordDecision).toHaveBeenCalledWith(artifact, 'approved', { requireBiometric: true });
  });

  test('tapping Reject calls recordDecision with the artifact and "rejected"', async () => {
    const { findByText, getByText } = await render(
      <ArtifactReviewScreen route={route} navigation={navigation} />
    );
    await findByText(artifact.title);

    await fireEvent.press(getByText('Reject'));

    await waitFor(() => expect(recordDecision).toHaveBeenCalledTimes(1));
    expect(recordDecision).toHaveBeenCalledWith(artifact, 'rejected', { requireBiometric: true });
  });

  test('a denied decision alerts the reviewer and renders no decision text', async () => {
    (recordDecision as jest.Mock).mockResolvedValue({ status: 'denied' });
    const alertSpy = jest.spyOn(Alert, 'alert').mockImplementation(() => {});

    const { findByText, getByText, queryByText } = await render(
      <ArtifactReviewScreen route={route} navigation={navigation} />
    );
    await findByText(artifact.title);

    await fireEvent.press(getByText('Approve'));

    await waitFor(() => expect(alertSpy).toHaveBeenCalled());
    expect(alertSpy).toHaveBeenCalledWith(
      'Not approved',
      'Biometric check did not pass — decision was not recorded.'
    );
    expect(queryByText(/Marked approved locally/)).toBeNull();
    expect(queryByText(/Marked rejected locally/)).toBeNull();
  });

  test('posting a comment while a file is targeted shows the targeted file path', async () => {
    const { findByText, getByText, getAllByText, getByPlaceholderText } = await render(
      <ArtifactReviewScreen route={route} navigation={navigation} />
    );
    await findByText(artifact.title);

    await fireEvent.press(getByText('Comment on this file'));
    await fireEvent.changeText(
      getByPlaceholderText('Leave a comment on this artifact'),
      'Looks good but double check the retry path'
    );
    await fireEvent.press(getByText('Post'));

    expect(getByText('Looks good but double check the retry path')).toBeTruthy();
    // art-1 already has a mock comment (cmt-1) targeting this same file, so
    // the new comment brings the count to two, not one.
    expect(getAllByText(`You · ${artifact.files[0].path}`)).toHaveLength(2);
  });
});
