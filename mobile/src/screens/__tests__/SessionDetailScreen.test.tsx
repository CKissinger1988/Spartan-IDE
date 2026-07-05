import { fireEvent, render } from '@testing-library/react-native';
import { SessionDetailScreen } from '../SessionDetailScreen';
import { captureFromCamera, pickFromLibrary } from '../../lib/imageCapture';

jest.mock('../../lib/imageCapture', () => ({
  captureFromCamera: jest.fn(),
  pickFromLibrary: jest.fn(),
}));

const navigation = { navigate: jest.fn(), goBack: jest.fn() };
const route = { params: { sessionId: 'sess-1' } };

beforeEach(() => {
  jest.clearAllMocks();
  (captureFromCamera as jest.Mock).mockResolvedValue({ cancelled: true, uri: null });
  (pickFromLibrary as jest.Mock).mockResolvedValue({ cancelled: true, uri: null });
});

function renderScreen() {
  return render(<SessionDetailScreen route={route as any} navigation={navigation as any} />);
}

// Image has no accessibility role by default, so *ByRole can't find thumbnails --
// query the host tree by RN's internal host component name instead.
function queryThumbnails(root: { queryAll: (predicate: (node: { type: unknown }) => boolean) => any[] } | null) {
  return root!.queryAll((node) => node.type === 'Image');
}

describe('SessionDetailScreen', () => {
  test('renders the session title and existing chat messages', async () => {
    const { getByText } = await renderScreen();

    expect(getByText('Fix checkout race condition')).toBeTruthy();
    expect(getByText('What triggered this fix?')).toBeTruthy();
    expect(getByText(/A support ticket describing double-decremented stock/)).toBeTruthy();
  });

  test('typing a message and sending it appends a new bubble and clears the input', async () => {
    const { getByPlaceholderText, getByText } = await renderScreen();

    const input = getByPlaceholderText('Message Leo');
    await fireEvent.changeText(input, 'Should we add a retry too?');
    await fireEvent.press(getByText('Send'));

    expect(getByText('Should we add a retry too?')).toBeTruthy();
    expect(input.props.value).toBe('');
  });

  test('sending a blank/whitespace-only message does not append a bubble', async () => {
    const { getByPlaceholderText, getByText, queryAllByText } = await renderScreen();

    const input = getByPlaceholderText('Message Leo');
    await fireEvent.changeText(input, '   ');
    await fireEvent.press(getByText('Send'));

    expect(queryAllByText('   ')).toHaveLength(0);
  });

  test('tapping "Take Photo" with the camera resolving a uri adds a thumbnail', async () => {
    (captureFromCamera as jest.Mock).mockResolvedValue({
      cancelled: false,
      uri: 'file:///camera-photo.jpg',
    });
    const { getByText, root } = await renderScreen();

    expect(queryThumbnails(root)).toHaveLength(0);

    await fireEvent.press(getByText('Take Photo'));

    const thumbnails = queryThumbnails(root);
    expect(thumbnails).toHaveLength(1);
    expect(thumbnails[0].props.source).toEqual({ uri: 'file:///camera-photo.jpg' });
  });

  test('tapping "Choose from Library" with the picker resolving a uri adds a thumbnail', async () => {
    (pickFromLibrary as jest.Mock).mockResolvedValue({
      cancelled: false,
      uri: 'file:///library-photo.jpg',
    });
    const { getByText, root } = await renderScreen();

    await fireEvent.press(getByText('Choose from Library'));

    const thumbnails = queryThumbnails(root);
    expect(thumbnails).toHaveLength(1);
    expect(thumbnails[0].props.source).toEqual({ uri: 'file:///library-photo.jpg' });
  });

  test('a cancelled camera capture does not add a thumbnail', async () => {
    const { getByText, root } = await renderScreen();

    await fireEvent.press(getByText('Take Photo'));

    expect(queryThumbnails(root)).toHaveLength(0);
  });
});
