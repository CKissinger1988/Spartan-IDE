import { NavigationContainer } from '@react-navigation/native';
import { createNativeStackNavigator } from '@react-navigation/native-stack';
import { Pressable, Text, View } from 'react-native';
import { ArtifactReviewScreen } from '../screens/ArtifactReviewScreen';
import { DecisionHistoryScreen } from '../screens/DecisionHistoryScreen';
import { InboxScreen } from '../screens/InboxScreen';
import { NewTaskScreen } from '../screens/NewTaskScreen';
import { SessionDetailScreen } from '../screens/SessionDetailScreen';
import { SettingsScreen } from '../screens/SettingsScreen';
import { C, navigationTheme } from '../theme';
import { navigationRef } from './navigationRef';
import { RootStackParamList } from './types';

const Stack = createNativeStackNavigator<RootStackParamList>();

export function RootNavigator() {
  return (
    <NavigationContainer ref={navigationRef} theme={navigationTheme}>
      <Stack.Navigator
        initialRouteName="Inbox"
        screenOptions={{
          headerStyle: { backgroundColor: C.s1 },
          headerTintColor: C.text,
          headerTitleStyle: { color: C.text },
          contentStyle: { backgroundColor: C.bg },
        }}
      >
        <Stack.Screen
          name="Inbox"
          component={InboxScreen}
          options={({ navigation }) => ({
            title: 'Inbox',
            headerRight: () => (
              <View style={{ flexDirection: 'row', gap: 16 }}>
                <Pressable onPress={() => navigation.navigate('NewTask')} hitSlop={8}>
                  <Text style={{ fontSize: 15, color: C.accent }}>Dictate</Text>
                </Pressable>
                <Pressable onPress={() => navigation.navigate('Settings')} hitSlop={8}>
                  <Text style={{ fontSize: 15, color: C.accent }}>Settings</Text>
                </Pressable>
              </View>
            ),
          })}
        />
        <Stack.Screen
          name="SessionDetail"
          component={SessionDetailScreen}
          options={{ title: 'Session' }}
        />
        <Stack.Screen
          name="ArtifactReview"
          component={ArtifactReviewScreen}
          options={{ title: 'Review' }}
        />
        <Stack.Screen
          name="Settings"
          component={SettingsScreen}
          options={{ title: 'Settings' }}
        />
        <Stack.Screen
          name="NewTask"
          component={NewTaskScreen}
          options={{ title: 'New Task' }}
        />
        <Stack.Screen
          name="DecisionHistory"
          component={DecisionHistoryScreen}
          options={{ title: 'Decision History' }}
        />
      </Stack.Navigator>
    </NavigationContainer>
  );
}
