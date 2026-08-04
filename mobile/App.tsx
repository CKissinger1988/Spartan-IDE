import { useEffect } from 'react';
import { StatusBar } from 'expo-status-bar';
import { SafeAreaProvider } from 'react-native-safe-area-context';
import { mockArtifacts } from './src/data/mockData';
import { BackendProvider } from './src/lib/backendContext';
import {
  addNotificationResponseListener,
  registerNotificationCategories,
} from './src/lib/notificationActions';
import { RootNavigator } from './src/navigation/RootNavigator';
import { ThemeProvider, useTheme } from './src/ThemeContext';

// Real §75.93 status-bar content style, made reactive -- previously
// hardcoded "light" (correct only for the dark theme, §50.3). Split into
// its own inner component since it needs `useTheme()`, which only works
// below `ThemeProvider`.
function ThemedStatusBar() {
  const { mode } = useTheme();
  return <StatusBar style={mode === 'light' ? 'dark' : 'light'} />;
}

export default function App() {
  useEffect(() => {
    registerNotificationCategories();

    // artifactLookup is mock-backed today (same honest boundary as every
    // other screen) — swap for a real session-store lookup once one exists.
    const subscription = addNotificationResponseListener((artifactId) =>
      mockArtifacts.find((a) => a.id === artifactId)
    );
    return () => subscription.remove();
  }, []);

  return (
    <BackendProvider>
      <ThemeProvider>
        <SafeAreaProvider>
          <ThemedStatusBar />
          <RootNavigator />
        </SafeAreaProvider>
      </ThemeProvider>
    </BackendProvider>
  );
}
