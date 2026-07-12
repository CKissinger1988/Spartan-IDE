import { useMemo } from 'react';
import { NativeStackScreenProps } from '@react-navigation/native-stack';
import { Pressable, StyleSheet, Text, TextInput, View } from 'react-native';
import { addLocalTask } from '../data/localTaskStore';
import { useVoiceCapture } from '../lib/voiceCapture';
import { RootStackParamList } from '../navigation/types';
import { useTheme } from '../ThemeContext';
import { ThemeColors } from '../theme';

type Props = NativeStackScreenProps<RootStackParamList, 'NewTask'>;

// §69.5's "Voice-to-task capture (v2)" — dictate, review the transcript,
// submit as a new task thread. See src/lib/voiceCapture.ts for the honest
// boundary: this needs a custom dev client to actually run, and has not
// been exercised against a real on-device speech engine here.
export function NewTaskScreen({ navigation }: Props) {
  const { colors } = useTheme();
  const styles = useMemo(() => makeStyles(colors), [colors]);
  const { isListening, transcript, error, start, stop, reset, setTranscript } = useVoiceCapture();

  const handleSubmit = () => {
    const title = transcript.trim();
    if (!title) return;
    addLocalTask(title);
    reset();
    navigation.goBack();
  };

  return (
    <View style={styles.container}>
      <Text style={styles.label}>Dictate a new task</Text>
      <TextInput
        style={styles.transcriptInput}
        multiline
        placeholder="Tap the mic and start speaking, or type here directly"
        placeholderTextColor={colors.textDim}
        value={transcript}
        onChangeText={setTranscript}
        editable={!isListening}
      />

      {error && <Text style={styles.error}>{error}</Text>}

      <Pressable
        style={[styles.micButton, isListening && styles.micButtonActive]}
        onPress={isListening ? stop : start}
      >
        <Text style={styles.micButtonText}>{isListening ? 'Stop' : 'Start dictation'}</Text>
      </Pressable>

      <Pressable
        style={[styles.submitButton, !transcript.trim() && styles.submitButtonDisabled]}
        onPress={handleSubmit}
        disabled={!transcript.trim()}
      >
        <Text style={styles.submitButtonText}>Add to Inbox</Text>
      </Pressable>

      <Text style={styles.note}>
        Local only — this needs a custom dev client (not Expo Go) to actually run on-device
        speech recognition, and has not been tested on real hardware. Adding a task here only
        stores it locally; there's no backend yet to submit it to.
      </Text>
    </View>
  );
}

function makeStyles(colors: ThemeColors) {
  return StyleSheet.create({
    container: {
      flex: 1,
      backgroundColor: colors.bg,
      padding: 16,
    },
    label: {
      fontSize: 16,
      fontWeight: '700',
      color: colors.text,
    },
    transcriptInput: {
      marginTop: 12,
      borderWidth: StyleSheet.hairlineWidth,
      borderColor: colors.borderLt,
      borderRadius: 8,
      padding: 12,
      minHeight: 100,
      textAlignVertical: 'top',
      color: colors.text,
      backgroundColor: colors.s1,
    },
    error: {
      marginTop: 8,
      color: colors.red,
      fontSize: 12,
    },
    micButton: {
      marginTop: 16,
      backgroundColor: colors.accent,
      borderRadius: 8,
      paddingVertical: 14,
      alignItems: 'center',
    },
    micButtonActive: {
      backgroundColor: colors.red,
    },
    micButtonText: {
      color: '#fff',
      fontWeight: '700',
    },
    submitButton: {
      marginTop: 12,
      backgroundColor: colors.green,
      borderRadius: 8,
      paddingVertical: 14,
      alignItems: 'center',
    },
    submitButtonDisabled: {
      opacity: 0.5,
    },
    submitButtonText: {
      color: '#fff',
      fontWeight: '700',
    },
    note: {
      marginTop: 20,
      color: colors.textDim,
      fontSize: 12,
    },
  });
}
