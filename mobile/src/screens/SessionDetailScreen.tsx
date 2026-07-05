import { useState } from 'react';
import { NativeStackScreenProps } from '@react-navigation/native-stack';
import {
  FlatList,
  Image,
  Pressable,
  StyleSheet,
  Text,
  TextInput,
  View,
} from 'react-native';
import { mockArtifacts, mockChatMessages, mockSessionThreads } from '../data/mockData';
import { captureFromCamera, pickFromLibrary } from '../lib/imageCapture';
import { RootStackParamList } from '../navigation/types';
import { ChatMessage, ImageAttachment } from '../types/domain';

type Props = NativeStackScreenProps<RootStackParamList, 'SessionDetail'>;

// §69.1's "Chat with Leo" (same conversation history as desktop/CLI, §46.2)
// plus a link into any pending Artifact for that session (§69.1's Approve/
// Reject on Diff Cards and Implementation Plans — the single highest-value
// mobile action named in the spec), plus §69.5's camera capture into
// artifacts as session context.
export function SessionDetailScreen({ route, navigation }: Props) {
  const { sessionId } = route.params;
  const session = mockSessionThreads.find((s) => s.id === sessionId);
  const pendingArtifact = mockArtifacts.find(
    (a) => a.sessionId === sessionId && a.approved === null
  );
  const [attachments, setAttachments] = useState<ImageAttachment[]>([]);
  const [chatMessages, setChatMessages] = useState<ChatMessage[]>(
    mockChatMessages.filter((m) => m.sessionId === sessionId)
  );
  const [draftMessage, setDraftMessage] = useState('');

  const handleSendMessage = () => {
    const text = draftMessage.trim();
    if (!text) return;

    setChatMessages((prev) => [
      ...prev,
      {
        id: `local-${Date.now()}`,
        sessionId,
        author: 'user',
        text,
        createdAt: new Date().toISOString(),
      },
    ]);
    setDraftMessage('');
  };

  const addAttachment = (uri: string) => {
    setAttachments((prev) => [
      ...prev,
      {
        id: `img-${Date.now()}`,
        sessionId,
        uri,
        caption: null,
        ocrText: null, // never populated — see src/lib/imageCapture.ts's note
        createdAt: new Date().toISOString(),
      },
    ]);
  };

  const handleTakePhoto = async () => {
    const result = await captureFromCamera();
    if (!result.cancelled && result.uri) addAttachment(result.uri);
  };

  const handlePickPhoto = async () => {
    const result = await pickFromLibrary();
    if (!result.cancelled && result.uri) addAttachment(result.uri);
  };

  return (
    <View style={styles.container}>
      <Text style={styles.header}>{session?.title ?? 'Session'}</Text>

      {pendingArtifact && (
        <Pressable
          style={styles.artifactBanner}
          onPress={() =>
            navigation.navigate('ArtifactReview', { artifactId: pendingArtifact.id })
          }
        >
          <Text style={styles.artifactBannerText}>
            Review needed: {pendingArtifact.title}
          </Text>
        </Pressable>
      )}

      <FlatList
        style={styles.messages}
        data={chatMessages}
        keyExtractor={(item) => item.id}
        renderItem={({ item }) => (
          <View
            style={[
              styles.bubble,
              item.author === 'user' ? styles.bubbleUser : styles.bubbleLeo,
            ]}
          >
            <Text
              style={[
                styles.bubbleText,
                item.author === 'user' && styles.bubbleTextUser,
              ]}
            >
              {item.text}
            </Text>
          </View>
        )}
      />

      <View style={styles.composerSection}>
        <View style={styles.composerRow}>
          <TextInput
            style={styles.composerInput}
            placeholder="Message Leo"
            value={draftMessage}
            onChangeText={setDraftMessage}
            multiline
          />
          <Pressable style={styles.composerSend} onPress={handleSendMessage}>
            <Text style={styles.composerSendText}>Send</Text>
          </Pressable>
        </View>
        <Text style={styles.composerNote}>
          Local only — no sync client exists yet to send this message to Leo.
        </Text>
      </View>

      <View style={styles.attachmentsSection}>
        <Text style={styles.attachmentsHeader}>Attachments</Text>
        {attachments.length > 0 && (
          <FlatList
            horizontal
            data={attachments}
            keyExtractor={(item) => item.id}
            renderItem={({ item }) => (
              <Image source={{ uri: item.uri }} style={styles.thumbnail} />
            )}
            contentContainerStyle={styles.thumbnailRow}
          />
        )}
        <View style={styles.attachmentActions}>
          <Pressable style={styles.attachmentButton} onPress={handleTakePhoto}>
            <Text style={styles.attachmentButtonText}>Take Photo</Text>
          </Pressable>
          <Pressable
            style={[styles.attachmentButton, styles.attachmentButtonSecondary]}
            onPress={handlePickPhoto}
          >
            <Text style={styles.attachmentButtonText}>Choose from Library</Text>
          </Pressable>
        </View>
        <Text style={styles.attachmentsNote}>
          Local only — not sent anywhere yet, and no text is extracted from the image (on-device
          OCR would need a native module this app doesn't include — see the README).
        </Text>
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  container: {
    flex: 1,
    backgroundColor: '#fff',
  },
  header: {
    fontSize: 18,
    fontWeight: '700',
    padding: 16,
  },
  artifactBanner: {
    marginHorizontal: 16,
    marginBottom: 8,
    padding: 12,
    borderRadius: 8,
    backgroundColor: '#fef3c7',
  },
  artifactBannerText: {
    color: '#92400e',
    fontWeight: '600',
  },
  messages: {
    flex: 1,
    paddingHorizontal: 16,
  },
  bubble: {
    maxWidth: '85%',
    borderRadius: 12,
    padding: 10,
    marginBottom: 8,
  },
  bubbleUser: {
    alignSelf: 'flex-end',
    backgroundColor: '#2563eb',
  },
  bubbleLeo: {
    alignSelf: 'flex-start',
    backgroundColor: '#f3f4f6',
  },
  bubbleText: {
    color: '#111827',
  },
  bubbleTextUser: {
    color: '#fff',
  },
  composerSection: {
    padding: 16,
    borderTopWidth: StyleSheet.hairlineWidth,
    borderTopColor: '#e5e7eb',
  },
  composerRow: {
    flexDirection: 'row',
    gap: 8,
    alignItems: 'flex-end',
  },
  composerInput: {
    flex: 1,
    borderWidth: StyleSheet.hairlineWidth,
    borderColor: '#d1d5db',
    borderRadius: 8,
    paddingHorizontal: 10,
    paddingVertical: 8,
    minHeight: 40,
    maxHeight: 100,
  },
  composerSend: {
    backgroundColor: '#2563eb',
    borderRadius: 8,
    paddingHorizontal: 14,
    paddingVertical: 10,
  },
  composerSendText: {
    color: '#fff',
    fontWeight: '700',
  },
  composerNote: {
    marginTop: 8,
    color: '#9ca3af',
    fontSize: 11,
  },
  attachmentsSection: {
    padding: 16,
    borderTopWidth: StyleSheet.hairlineWidth,
    borderTopColor: '#e5e7eb',
  },
  attachmentsHeader: {
    fontSize: 14,
    fontWeight: '700',
  },
  thumbnailRow: {
    gap: 8,
    marginTop: 8,
  },
  thumbnail: {
    width: 64,
    height: 64,
    borderRadius: 8,
  },
  attachmentActions: {
    flexDirection: 'row',
    gap: 10,
    marginTop: 10,
  },
  attachmentButton: {
    backgroundColor: '#2563eb',
    borderRadius: 8,
    paddingVertical: 10,
    paddingHorizontal: 12,
  },
  attachmentButtonSecondary: {
    backgroundColor: '#6b7280',
  },
  attachmentButtonText: {
    color: '#fff',
    fontWeight: '700',
    fontSize: 12,
  },
  attachmentsNote: {
    marginTop: 8,
    color: '#9ca3af',
    fontSize: 11,
  },
});
