import React from 'react';
import { fireEvent, render } from '@testing-library/react-native';
import { NewTaskScreen } from '../NewTaskScreen';
import { useVoiceCapture } from '../../lib/voiceCapture';
import { addLocalTask } from '../../data/localTaskStore';

jest.mock('../../lib/voiceCapture', () => ({
  useVoiceCapture: jest.fn(),
}));
jest.mock('../../data/localTaskStore', () => ({
  addLocalTask: jest.fn(),
}));

const mockedUseVoiceCapture = useVoiceCapture as jest.Mock;

function makeVoiceCapture(overrides: Partial<ReturnType<typeof useVoiceCapture>> = {}) {
  return {
    isListening: false,
    transcript: '',
    error: null,
    start: jest.fn(),
    stop: jest.fn(),
    reset: jest.fn(),
    setTranscript: jest.fn(),
    ...overrides,
  };
}

async function renderScreen() {
  const navigation = { navigate: jest.fn(), goBack: jest.fn() };
  const route = { params: undefined };
  const utils = await render(<NewTaskScreen navigation={navigation as any} route={route as any} />);
  return { navigation, route, ...utils };
}

describe('NewTaskScreen', () => {
  afterEach(() => {
    jest.clearAllMocks();
  });

  test('with a non-empty transcript, Submit is enabled and adds the task then goes back', async () => {
    const voiceCapture = makeVoiceCapture({ transcript: 'Fix the login bug' });
    mockedUseVoiceCapture.mockReturnValue(voiceCapture);

    const { getByText, navigation } = await renderScreen();
    const submit = getByText('Add to Inbox');

    expect(submit.parent?.props.accessibilityState?.disabled).toBeFalsy();

    fireEvent.press(submit);

    expect(addLocalTask).toHaveBeenCalledWith('Fix the login bug');
    expect(voiceCapture.reset).toHaveBeenCalled();
    expect(navigation.goBack).toHaveBeenCalled();
  });

  test('with an empty transcript, Submit is disabled and pressing it does nothing', async () => {
    mockedUseVoiceCapture.mockReturnValue(makeVoiceCapture({ transcript: '' }));

    const { getByText, navigation } = await renderScreen();
    const submit = getByText('Add to Inbox');

    expect(submit.parent?.props.accessibilityState?.disabled).toBe(true);

    fireEvent.press(submit);

    expect(addLocalTask).not.toHaveBeenCalled();
    expect(navigation.goBack).not.toHaveBeenCalled();
  });

  test('pressing "Start dictation" calls start()', async () => {
    const voiceCapture = makeVoiceCapture();
    mockedUseVoiceCapture.mockReturnValue(voiceCapture);

    const { getByText } = await renderScreen();
    fireEvent.press(getByText('Start dictation'));

    expect(voiceCapture.start).toHaveBeenCalled();
  });

  test('when isListening is true, the button shows "Stop" and pressing it calls stop()', async () => {
    const voiceCapture = makeVoiceCapture({ isListening: true, transcript: 'partial words' });
    mockedUseVoiceCapture.mockReturnValue(voiceCapture);

    const { getByText, queryByText } = await renderScreen();
    expect(queryByText('Start dictation')).toBeNull();

    fireEvent.press(getByText('Stop'));

    expect(voiceCapture.stop).toHaveBeenCalled();
  });

  test('when error is set, the error text renders', async () => {
    mockedUseVoiceCapture.mockReturnValue(
      makeVoiceCapture({ error: 'Microphone/speech permission not granted.' })
    );

    const { getByText } = await renderScreen();

    expect(getByText('Microphone/speech permission not granted.')).toBeTruthy();
  });
});
