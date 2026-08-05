// React context providing a live BackendClient to the entire app tree.
// The provider attempts to connect on mount; if the devserver isn't
// reachable, `client` stays null and every screen falls back to its
// existing mock-data path. This is the exact same "optional by
// construction" pattern web/src/backendClient.ts already establishes.

import React, { createContext, useCallback, useContext, useEffect, useState } from 'react';
import { BackendClient } from './backendClient';
import { DEFAULT_BACKEND_ENDPOINT, getBackendEndpoint, setBackendEndpoint } from './backendEndpoint';
import { getBackendPairingToken, setBackendPairingToken } from './backendPairing';

interface BackendContextValue {
  /** The live backend client, or null if no devserver is reachable. */
  client: BackendClient | null;
  /** True while the initial connection attempt is in flight. */
  connecting: boolean;
  endpoint: string;
  error: string | null;
  reconnect: () => Promise<void>;
  updateEndpoint: (endpoint: string, pairingToken: string) => Promise<void>;
}

const BackendContext = createContext<BackendContextValue>({
  client: null,
  connecting: true,
  endpoint: DEFAULT_BACKEND_ENDPOINT,
  error: null,
  reconnect: async () => {},
  updateEndpoint: async () => {},
});

/**
 * Provider that attempts to connect to spartan-devserver on mount.
 * Pass `baseUrl` to target a specific devserver (default:
 * `http://127.0.0.1:3000`). When the connection fails or no devserver is
 * present, `client` stays null and every consumer falls back to mock data.
 */
export function BackendProvider({
  children,
}: {
  children: React.ReactNode;
}) {
  const [client, setClient] = useState<BackendClient | null>(null);
  const [connecting, setConnecting] = useState(true);
  const [endpoint, setEndpoint] = useState(DEFAULT_BACKEND_ENDPOINT);
  const [error, setError] = useState<string | null>(null);
  const [connectionAttempt, setConnectionAttempt] = useState(0);

  useEffect(() => {
    let cancelled = false;
    let connectedClient: BackendClient | null = null;
    setConnecting(true);
    setError(null);
    Promise.all([getBackendEndpoint(), getBackendPairingToken()])
      .then(([storedEndpoint, pairingToken]) => {
        if (cancelled) return null;
        setEndpoint(storedEndpoint);
        return BackendClient.connect({ baseUrl: storedEndpoint, pairingToken });
      })
      .then((nextClient) => {
        if (!nextClient) return;
        connectedClient = nextClient;
        if (cancelled) {
          nextClient.close();
          return;
        }
        setClient(nextClient);
      })
      .catch((reason: unknown) => {
        if (!cancelled) {
          setClient(null);
          setError(reason instanceof Error ? reason.message : 'Connection failed');
        }
      })
      .finally(() => {
        if (!cancelled) setConnecting(false);
      });
    return () => {
      cancelled = true;
      connectedClient?.close();
    };
  }, [connectionAttempt]);

  const reconnect = useCallback(async () => {
    setConnectionAttempt((attempt) => attempt + 1);
  }, []);

  const updateEndpoint = useCallback(async (nextEndpoint: string, pairingToken: string) => {
    const saved = await setBackendEndpoint(nextEndpoint);
    await setBackendPairingToken(pairingToken);
    setEndpoint(saved);
    setConnectionAttempt((attempt) => attempt + 1);
  }, []);

  return (
    <BackendContext.Provider value={{ client, connecting, endpoint, error, reconnect, updateEndpoint }}>
      {children}
    </BackendContext.Provider>
  );
}

/** Access the backend client. Returns null when no devserver is connected. */
export function useBackend(): BackendClient | null {
  return useContext(BackendContext).client;
}

/** Whether the initial connection attempt is still in flight. */
export function useBackendConnecting(): boolean {
  return useContext(BackendContext).connecting;
}

/** Connection state and controls for the mobile companion's Settings screen. */
export function useBackendConnection(): Omit<BackendContextValue, 'client'> {
  const { client: _client, ...connection } = useContext(BackendContext);
  return connection;
}
