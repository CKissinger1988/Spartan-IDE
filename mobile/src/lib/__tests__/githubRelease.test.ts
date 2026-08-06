import { checkMobileRelease, isNewerVersion } from '../githubRelease';

describe('GitHub mobile release checks', () => {
  afterEach(() => jest.restoreAllMocks());

  test('orders stable releases after matching prereleases', () => {
    expect(isNewerVersion('v0.2.0', '0.2.0-beta.1')).toBe(true);
    expect(isNewerVersion('0.2.0-beta.1', '0.2.0')).toBe(false);
    expect(isNewerVersion('0.2.1', '0.2.0')).toBe(true);
    expect(isNewerVersion('not-a-version', '0.2.0')).toBe(false);
  });

  test('accepts only GitHub release and APK URLs', async () => {
    jest.spyOn(globalThis, 'fetch').mockResolvedValue({
      ok: true,
      json: async () => ({
        tag_name: 'v0.2.0',
        html_url: 'https://github.com/Spartan-Software-Enterprises/Spartan-IDE/releases/tag/v0.2.0',
        assets: [{ name: 'spartan-mobile-ide-0.2.0-debug.apk', browser_download_url: 'https://github.com/a.apk' }],
      }),
    } as Response);
    await expect(checkMobileRelease('0.2.0-beta.1')).resolves.toMatchObject({
      updateAvailable: true,
      androidDownloadUrl: 'https://github.com/a.apk',
    });
  });

  test('rejects a release link outside GitHub', async () => {
    jest.spyOn(globalThis, 'fetch').mockResolvedValue({
      ok: true,
      json: async () => ({ tag_name: 'v0.2.0', html_url: 'https://example.invalid/release' }),
    } as Response);
    await expect(checkMobileRelease('0.1.0')).rejects.toThrow('safe release URL');
  });
});
