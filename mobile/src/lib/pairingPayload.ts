export type PairingPayload =
  | { kind: 'private'; endpoint: string; pairingToken: string }
  | { kind: 'cloud'; endpoint: string; pairingToken: null };

/** Parse the compact, versioned payload emitted by both Spartan server CLIs. */
export function parsePairingPayload(value: string): PairingPayload | null {
  try {
    const url = new URL(value.trim());
    if (url.protocol !== 'spartan:' || url.hostname !== 'pair' || url.pathname !== '/v1') return null;
    const kind = url.searchParams.get('kind');
    const endpoint = url.searchParams.get('endpoint');
    if (!endpoint || (kind !== 'private' && kind !== 'cloud')) return null;
    const normalizedEndpoint = new URL(endpoint).origin;
    if (kind === 'cloud') {
      if (!normalizedEndpoint.startsWith('https://')) return null;
      return { kind, endpoint: normalizedEndpoint, pairingToken: null };
    }
    const pairingToken = url.searchParams.get('pairing');
    if (!pairingToken) return null;
    return { kind, endpoint: normalizedEndpoint, pairingToken };
  } catch {
    return null;
  }
}
