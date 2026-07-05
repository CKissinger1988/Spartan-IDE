import { localModelClient } from '../localModel';

describe('localModelClient stub', () => {
  test('isAvailable honestly reports no on-device model is wired up', async () => {
    await expect(localModelClient.isAvailable()).resolves.toBe(false);
  });

  test('ask rejects with an explanatory error instead of faking an answer', async () => {
    await expect(localModelClient.ask('What changed?', 'diff context')).rejects.toThrow(
      /No on-device model is wired up in this build/
    );
  });
});
