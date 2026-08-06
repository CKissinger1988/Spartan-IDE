/** GitHub Releases client shared by the mobile update surface. Android cannot
 * replace an installed APK silently, so this only discovers a newer signed
 * release and returns its GitHub-owned install URL for an explicit user tap. */

const REPOSITORY = 'Spartan-Software-Enterprises/Spartan-IDE';
const RELEASE_URL = `https://api.github.com/repos/${REPOSITORY}/releases/latest`;

export type MobileReleaseCheck = {
  currentVersion: string;
  latestVersion: string;
  updateAvailable: boolean;
  releaseUrl: string;
  androidDownloadUrl?: string;
};

function versionParts(value: string): { core: number[]; prerelease?: string } | null {
  const match = value.trim().replace(/^v/, '').match(/^(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z.-]+))?$/);
  if (!match) return null;
  return { core: [Number(match[1]), Number(match[2]), Number(match[3])], prerelease: match[4] };
}

/** True only when `latest` is strictly newer. Invalid versions never trigger
 * an install prompt. Stable releases sort after their equivalent prerelease. */
export function isNewerVersion(latest: string, current: string): boolean {
  const latestParts = versionParts(latest);
  const currentParts = versionParts(current);
  if (!latestParts || !currentParts) return false;
  for (let index = 0; index < 3; index += 1) {
    if (latestParts.core[index] !== currentParts.core[index]) {
      return latestParts.core[index] > currentParts.core[index];
    }
  }
  if (!latestParts.prerelease && currentParts.prerelease) return true;
  if (latestParts.prerelease && !currentParts.prerelease) return false;
  return Boolean(latestParts.prerelease && currentParts.prerelease && latestParts.prerelease > currentParts.prerelease);
}

export async function checkMobileRelease(currentVersion: string): Promise<MobileReleaseCheck> {
  const response = await fetch(RELEASE_URL, {
    headers: { Accept: 'application/vnd.github+json' },
  });
  if (!response.ok) throw new Error(`GitHub release check failed (${response.status})`);
  const release: unknown = await response.json();
  if (!release || typeof release !== 'object') throw new Error('GitHub returned an invalid release response');
  const record = release as { tag_name?: unknown; html_url?: unknown; assets?: unknown };
  if (typeof record.tag_name !== 'string' || typeof record.html_url !== 'string' || !record.html_url.startsWith('https://github.com/')) {
    throw new Error('GitHub release response is missing a safe release URL');
  }
  const assets = Array.isArray(record.assets) ? record.assets : [];
  const apk = assets.find((asset): asset is { name: string; browser_download_url: string } => {
    if (!asset || typeof asset !== 'object') return false;
    const value = asset as { name?: unknown; browser_download_url?: unknown };
    return typeof value.name === 'string'
      && value.name.endsWith('.apk')
      && typeof value.browser_download_url === 'string'
      && value.browser_download_url.startsWith('https://github.com/');
  });
  return {
    currentVersion,
    latestVersion: record.tag_name.replace(/^v/, ''),
    updateAvailable: isNewerVersion(record.tag_name, currentVersion),
    releaseUrl: record.html_url,
    androidDownloadUrl: apk?.browser_download_url,
  };
}
