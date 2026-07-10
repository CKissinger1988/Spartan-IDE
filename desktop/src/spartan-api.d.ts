export {};

declare global {
  interface Window {
    spartan: {
      call: (method: string, params?: Record<string, unknown>) => Promise<unknown>;
    };
  }
}
