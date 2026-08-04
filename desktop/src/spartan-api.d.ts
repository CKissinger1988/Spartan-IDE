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
      onCloseRequested: (listener: () => void) => () => void;
      confirmClose: () => void;
      onUpdateAvailable: (listener: (info: { version: string; releaseDate?: string; releaseNotes?: string | null }) => void) => () => void;
      onUpdateNotAvailable: (listener: (info: { version: string }) => void) => () => void;
      onUpdateDownloadProgress: (listener: (info: { percent: number; transferred: number; total: number }) => void) => () => void;
      onUpdateDownloaded: (listener: (info: { version: string }) => void) => () => void;
      onUpdateError: (listener: (info: { message: string }) => void) => () => void;
    };
  }
}
