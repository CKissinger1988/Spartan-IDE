import { parsePairingPayload } from '../pairingPayload';

describe('parsePairingPayload', () => {
  test('parses a private-server endpoint and pairing secret', () => {
    expect(
      parsePairingPayload(
        'spartan://pair/v1?kind=private&endpoint=http%3A%2F%2F192.168.1.20%3A4400&pairing=secret'
      )
    ).toEqual({ kind: 'private', endpoint: 'http://192.168.1.20:4400', pairingToken: 'secret' });
  });

  test('parses an HTTPS cloud endpoint without a bearer credential', () => {
    expect(
      parsePairingPayload('spartan://pair/v1?kind=cloud&endpoint=https%3A%2F%2Fcloud.example.com')
    ).toEqual({ kind: 'cloud', endpoint: 'https://cloud.example.com', pairingToken: null });
  });

  test('refuses malformed, unversioned, and insecure cloud payloads', () => {
    expect(parsePairingPayload('spartan://pair/v2?kind=private')).toBeNull();
    expect(
      parsePairingPayload('spartan://pair/v1?kind=cloud&endpoint=http%3A%2F%2Fcloud.example.com')
    ).toBeNull();
    expect(parsePairingPayload('https://cloud.example.com')).toBeNull();
  });
});
