// React context providing a live BackendClient to the entire app tree.
// The provider attempts to connect on mount; if the devserver isn't
// reachable, `client` stays null and every screen falls back to its
// existing mock-data path. This is the exact same "optional by
// construction" pattern web/src/backendClient.ts already establishes.

import React, { createContext, useContext, useEffect, useState } from 'react';
import { BackendClient } from './backendClient';

interface BackendContextValue {
  /** The live backend client, or null if no devserver is reachable. */
  client: BackendClient | null;
  /** True while the initial connection attempt is in flight. */
  connecting: boolean;
}

const BackendContext = createContext<BackendContextValue>({
  client: null,
  connecting: true,
});

/**
 * Provider that attempts to connect to spartan-devserver on mount.
 * Pass `baseUrl` to target a specific devserver (default:
 * `http://127.0.0.1:3000`). When the connection fails or no devserver is
 * present, `client` stays null and every consumer falls back to mock data.
 */
export function BackendProvider({
  baseUrl,
  children,
}: {
  baseUrl?: string;
  children: React.ReactNode;
}) {
  const [client, setClient] = useState<BackendClient | null>(null);
  const [connecting, setConnecting] = useState(true);

  useEffect(() => {
    let cancelled = false;
    BackendClient.connect({ baseUrl })
      .then((c) => {
        if (!cancelled) {
          setClient(c);
          setConnecting(false);
        } else {
          c.close();
        }
      })
      .catch(() => {
        if (!cancelled) setConnecting(false);
      });
    return () => {
      cancelled = true;
    };
  }, [baseUrl]);

  // Close the WebSocket on unmount.
  useEffect(() => {
    return () => {
      client?.close();
    };
  }, [client]);

  return (
    <BackendContext.Provider value={{ client, connecting }}>
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
