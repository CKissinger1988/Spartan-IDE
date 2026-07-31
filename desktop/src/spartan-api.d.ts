export {};

declare global {
  interface Window {
    spartan: {
      call: (method: string, params?: Record<string, unknown>) => Promise<unknown>;
      onEvent: (listener: (event: string, data: unknown) => void) => () => void;
      openCrashReportsFolder: () => Promise<unknown>;
      openRepositoryPage: () => Promise<unknown>;
      openPullRequestUrl: (url: string) => Promise<unknown>;
      openProject: (root: string) => Promise<unknown>;
      pickFolder: () => Promise<unknown>;
      pickFile: (filters?: { name: string; extensions: string[] }[]) => Promise<unknown>;
    };
  }
}
