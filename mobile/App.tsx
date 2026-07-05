import { useEffect } from 'react';
import { StatusBar } from 'expo-status-bar';
import { SafeAreaProvider } from 'react-native-safe-area-context';
import { mockArtifacts } from './src/data/mockData';
import {
  addNotificationResponseListener,
  registerNotificationCategories,
} from './src/lib/notificationActions';
import { RootNavigator } from './src/navigation/RootNavigator';

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
    <SafeAreaProvider>
      {/* light content for the dark theme (§50.3) -- see src/theme.ts */}
      <StatusBar style="light" />
      <RootNavigator />
    </SafeAreaProvider>
  );
}
