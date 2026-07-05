import { createNavigationContainerRef } from '@react-navigation/native';
import { RootStackParamList } from './types';

// Lets code outside the component tree (the notification response handler
// in src/lib/notificationActions.ts) navigate imperatively — e.g. opening
// ArtifactReview when a destructive-class artifact's notification is
// tapped rather than one of its action buttons.
export const navigationRef = createNavigationContainerRef<RootStackParamList>();
