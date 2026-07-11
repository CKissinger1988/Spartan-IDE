import { useEffect, useMemo, useState } from 'react';
import { NativeStackScreenProps } from '@react-navigation/native-stack';
import { Alert, ScrollView, StyleSheet, Text, TextInput, View } from 'react-native';
import { Pressable } from 'react-native';
import { mockArtifactComments, mockArtifacts } from '../data/mockData';
import { cacheArtifact, getCachedArtifact } from '../lib/edgeCache';
import { recordDecision } from '../lib/decisionActions';
import { DiffLineType, parseDiff } from '../lib/diffHighlight';
import { localModelClient } from '../lib/localModel';
import { RootStackParamList } from '../navigation/types';
import { useTheme } from '../ThemeContext';
import { MONO_FONT_FAMILY, ThemeColors } from '../theme';
import { Artifact, ArtifactComment } from '../types/domain';

type Props = NativeStackScreenProps<RootStackParamList, 'ArtifactReview'>;

// §69.1's read-only diff viewer plus Approve/Reject — "not an editor, a
// viewer." No local decision state is treated as final here: §69.5's
// offline-first rule ("approved as of <state the reviewer actually saw>")
// belongs in the real sync client this screen will eventually call, not
// invented here as a fake success path.
export function ArtifactReviewScreen({ route }: Props) {
  const { colors } = useTheme();
  const styles = useMemo(() => makeStyles(colors), [colors]);
  const patchLineStyles: Record<DiffLineType, object> = useMemo(
    () => ({
      add: styles.patchAdd,
      remove: styles.patchRemove,
      hunk: styles.patchHunk,
      context: styles.patchContext,
    }),
    [styles]
  );
  const { artifactId } = route.params;
  const [artifact, setArtifact] = useState<Artifact | null | undefined>(undefined);
  const [fromCache, setFromCache] = useState(false);
  const [decision, setDecision] = useState<'approved' | 'rejected' | null>(null);
  const [comments, setComments] = useState<ArtifactComment[]>(
    mockArtifactComments.filter((c) => c.artifactId === artifactId)
  );
  const [draftComment, setDraftComment] = useState('');
  const [commentTarget, setCommentTarget] = useState<string | null>(null);
  const [gating, setGating] = useState(false);

  useEffect(() => {
    // §69.5's edge-cached repo context: try the live (mock, for now) source
    // first and cache what it returns; fall back to whatever was cached
    // from a previous view if the live lookup comes up empty — the same
    // shape a real "no connection, session store unreachable" path would
    // take once one exists.
    const live = mockArtifacts.find((a) => a.id === artifactId);
    if (live) {
      setArtifact(live);
      setFromCache(false);
      cacheArtifact(live);
      return;
    }

    getCachedArtifact(artifactId).then((cached) => {
      setArtifact(cached);
      setFromCache(cached !== null);
    });
  }, [artifactId]);

  if (artifact === undefined) {
    return (
      <View style={styles.container}>
        <Text style={styles.title}>Loading…</Text>
      </View>
    );
  }

  if (!artifact) {
    return (
      <View style={styles.container}>
        <Text style={styles.title}>Artifact not found — not live, and nothing cached for it either.</Text>
      </View>
    );
  }

  const handleDecision = async (next: 'approved' | 'rejected') => {
    setGating(true);
    const outcome = await recordDecision(artifact, next, { requireBiometric: true });
    setGating(false);

    if (outcome.status === 'denied') {
      Alert.alert('Not approved', 'Biometric check did not pass — decision was not recorded.');
      return;
    }

    setDecision(next);

    if (outcome.status === 'biometric_unavailable') {
      // §69.5 names this as an *addition* to the security posture, not a
      // hard requirement that locks reviewers out of a device with no
      // biometrics enrolled — falls back to recording the decision plainly.
      Alert.alert('Biometric check unavailable', outcome.reason);
    }

    const queued = outcome.queued;
    Alert.alert(
      queued
        ? next === 'approved'
          ? 'Approved (queued)'
          : 'Rejected (queued)'
        : next === 'approved'
          ? 'Approved'
          : 'Rejected',
      queued
        ? 'No connection — queued on-device and will sync once one exists to replay to.'
        : 'Local only — no sync client exists yet to send this decision back to the session store.'
    );
  };

  const handleAddComment = () => {
    const text = draftComment.trim();
    if (!text) return;

    setComments((prev) => [
      ...prev,
      {
        id: `local-${Date.now()}`,
        artifactId: artifact.id,
        filePath: commentTarget,
        author: 'You',
        text,
        createdAt: new Date().toISOString(),
      },
    ]);
    setDraftComment('');
    setCommentTarget(null);
  };

  const handleAskLocalModel = async () => {
    const available = await localModelClient.isAvailable();
    if (!available) {
      Alert.alert(
        'No on-device model',
        'This build has no local model wired up — see src/lib/localModel.ts for what a real ' +
          'integration needs and why it is not built yet.'
      );
      return;
    }
    // Unreachable today (isAvailable() always returns false), kept so the
    // call site is correct once a real client replaces the stub.
    const answer = await localModelClient.ask('Explain this diff', artifact.summary);
    Alert.alert('Local model', answer);
  };

  return (
    <ScrollView style={styles.container}>
      {fromCache && (
        <Text style={styles.cacheBanner}>
          Showing a cached copy from your last visit — the live source wasn't reachable.
        </Text>
      )}
      <Text style={styles.title}>{artifact.title}</Text>
      <Text style={styles.summary}>{artifact.summary}</Text>

      {artifact.files.map((file) => (
        <View key={file.path} style={styles.fileBlock}>
          <Text style={styles.filePath}>{file.path}</Text>
          <Text style={styles.fileStats}>
            +{file.additions} -{file.deletions}
          </Text>
          <ScrollView horizontal style={styles.patchScroll}>
            <View style={styles.patchLines}>
              {parseDiff(file.patch).map((line, index) => (
                <Text key={index} style={[styles.patchLine, patchLineStyles[line.type]]}>
                  {line.text}
                </Text>
              ))}
            </View>
          </ScrollView>
          <Pressable style={styles.commentOnFile} onPress={() => setCommentTarget(file.path)}>
            <Text style={styles.commentOnFileText}>Comment on this file</Text>
          </Pressable>
        </View>
      ))}

      <View style={styles.actions}>
        <Pressable
          style={[styles.button, styles.rejectButton]}
          disabled={gating}
          onPress={() => handleDecision('rejected')}
        >
          <Text style={styles.buttonText}>Reject</Text>
        </Pressable>
        <Pressable
          style={[styles.button, styles.approveButton]}
          disabled={gating}
          onPress={() => handleDecision('approved')}
        >
          <Text style={styles.buttonText}>Approve</Text>
        </Pressable>
      </View>

      {decision && (
        <Text style={styles.decisionNote}>
          Marked {decision} locally. Not yet sent anywhere — see the note above.
        </Text>
      )}

      <View style={styles.commentsSection}>
        <Text style={styles.commentsHeader}>Comments</Text>
        {comments.map((comment) => (
          <View key={comment.id} style={styles.commentRow}>
            <Text style={styles.commentAuthor}>
              {comment.author}
              {comment.filePath ? ` · ${comment.filePath}` : ''}
            </Text>
            <Text style={styles.commentText}>{comment.text}</Text>
          </View>
        ))}
        {commentTarget && (
          <View style={styles.commentTargetRow}>
            <Text style={styles.commentTargetText}>Commenting on: {commentTarget}</Text>
            <Pressable onPress={() => setCommentTarget(null)}>
              <Text style={styles.commentTargetCancel}>Cancel</Text>
            </Pressable>
          </View>
        )}
        <View style={styles.commentInputRow}>
          <TextInput
            style={styles.commentInput}
            placeholder="Leave a comment on this artifact"
            placeholderTextColor={colors.textDim}
            value={draftComment}
            onChangeText={setDraftComment}
            multiline
          />
          <Pressable style={styles.commentSubmit} onPress={handleAddComment}>
            <Text style={styles.commentSubmitText}>Post</Text>
          </Pressable>
        </View>
        <Text style={styles.commentsNote}>
          Local only — no sync client exists yet to send comments back to the session store.
        </Text>
      </View>

      <Pressable style={styles.localModelButton} onPress={handleAskLocalModel}>
        <Text style={styles.localModelButtonText}>Ask about this diff (on-device)</Text>
      </Pressable>
      <Text style={styles.commentsNote}>
        No on-device model is wired up in this build — this button demonstrates the intended
        affordance and reports that honestly rather than faking an answer.
      </Text>
    </ScrollView>
  );
}

function makeStyles(colors: ThemeColors) {
  return StyleSheet.create({
    container: {
      flex: 1,
      backgroundColor: colors.bg,
      padding: 16,
    },
    title: {
      fontSize: 18,
      fontWeight: '700',
      color: colors.text,
    },
    summary: {
      marginTop: 6,
      color: colors.textMid,
    },
    fileBlock: {
      marginTop: 16,
      borderWidth: StyleSheet.hairlineWidth,
      borderColor: colors.border,
      borderRadius: 8,
      overflow: 'hidden',
    },
    filePath: {
      fontFamily: MONO_FONT_FAMILY,
      fontWeight: '600',
      paddingHorizontal: 10,
      paddingTop: 8,
      color: colors.text,
    },
    fileStats: {
      fontFamily: MONO_FONT_FAMILY,
      color: colors.textMid,
      paddingHorizontal: 10,
      paddingBottom: 6,
    },
    patchScroll: {
      backgroundColor: colors.s1,
    },
    patchLines: {
      paddingVertical: 10,
    },
    patchLine: {
      fontFamily: MONO_FONT_FAMILY,
      fontSize: 12,
      color: colors.text,
      paddingHorizontal: 10,
    },
    patchAdd: {
      color: colors.green,
    },
    patchRemove: {
      color: colors.red,
    },
    patchHunk: {
      color: colors.textDim,
    },
    patchContext: {
      color: colors.text,
    },
    commentOnFile: {
      paddingVertical: 8,
      paddingHorizontal: 10,
    },
    commentOnFileText: {
      color: colors.accent,
      fontSize: 12,
      fontWeight: '600',
    },
    commentTargetRow: {
      flexDirection: 'row',
      justifyContent: 'space-between',
      alignItems: 'center',
      backgroundColor: colors.accentBg,
      borderRadius: 8,
      padding: 8,
      marginTop: 12,
    },
    commentTargetText: {
      color: colors.accent,
      fontSize: 12,
      fontFamily: MONO_FONT_FAMILY,
      flexShrink: 1,
    },
    commentTargetCancel: {
      color: colors.accent,
      fontSize: 12,
      fontWeight: '700',
      marginLeft: 8,
    },
    actions: {
      flexDirection: 'row',
      gap: 12,
      marginTop: 24,
    },
    button: {
      flex: 1,
      paddingVertical: 12,
      borderRadius: 8,
      alignItems: 'center',
    },
    rejectButton: {
      backgroundColor: colors.red,
    },
    approveButton: {
      backgroundColor: colors.green,
    },
    buttonText: {
      color: '#fff',
      fontWeight: '700',
    },
    decisionNote: {
      marginTop: 12,
      color: colors.textMid,
      fontSize: 12,
      textAlign: 'center',
    },
    cacheBanner: {
      backgroundColor: colors.accentBg,
      color: colors.accent,
      fontSize: 12,
      padding: 10,
      borderRadius: 8,
      marginBottom: 12,
    },
    commentsSection: {
      marginTop: 28,
      marginBottom: 24,
    },
    commentsHeader: {
      fontSize: 15,
      fontWeight: '700',
      marginBottom: 8,
      color: colors.text,
    },
    commentRow: {
      paddingVertical: 8,
      borderBottomWidth: StyleSheet.hairlineWidth,
      borderBottomColor: colors.border,
    },
    commentAuthor: {
      fontSize: 12,
      fontWeight: '600',
      color: colors.textMid,
    },
    commentText: {
      marginTop: 2,
      color: colors.text,
    },
    commentInputRow: {
      flexDirection: 'row',
      gap: 8,
      marginTop: 12,
      alignItems: 'flex-end',
    },
    commentInput: {
      flex: 1,
      borderWidth: StyleSheet.hairlineWidth,
      borderColor: colors.borderLt,
      borderRadius: 8,
      paddingHorizontal: 10,
      paddingVertical: 8,
      minHeight: 40,
      maxHeight: 100,
      color: colors.text,
      backgroundColor: colors.s1,
    },
    commentSubmit: {
      backgroundColor: colors.accent,
      borderRadius: 8,
      paddingHorizontal: 14,
      paddingVertical: 10,
    },
    commentSubmitText: {
      color: '#fff',
      fontWeight: '700',
    },
    commentsNote: {
      marginTop: 8,
      color: colors.textDim,
      fontSize: 12,
    },
    localModelButton: {
      marginBottom: 4,
      backgroundColor: colors.teal,
      borderRadius: 8,
      paddingVertical: 12,
      alignItems: 'center',
    },
    localModelButtonText: {
      color: '#fff',
      fontWeight: '700',
    },
  });
}
