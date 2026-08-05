import { Pressable, StyleSheet, Text, View } from 'react-native';
import { useTheme } from '../ThemeContext';

export function FirstRunScreen({ onContinue }: { onContinue: () => void }) {
  const { colors } = useTheme();
  return (
    <View style={[styles.container, { backgroundColor: colors.bg }]}> 
      <Text style={[styles.brand, { color: colors.accent }]}>SPARTAN MOBILE</Text>
      <Text style={[styles.title, { color: colors.text }]}>Connect, review, and stay in control.</Text>
      <Text style={[styles.body, { color: colors.textMid }]}> 
        Pair with your private Linux devserver by scanning its QR code in Settings. For WAN access,
        use a TLS-protected endpoint or an SSH tunnel you control.
      </Text>
      <Text style={[styles.body, { color: colors.textMid }]}> 
        Spartan checks GitHub Releases for updates. Installing an Android APK always stays an explicit
        Android confirmation; this app never replaces itself silently.
      </Text>
      <Pressable style={[styles.button, { backgroundColor: colors.accent }]} onPress={onContinue} testID="first-run-continue">
        <Text style={styles.buttonText}>Continue to Inbox</Text>
      </Pressable>
    </View>
  );
}

const styles = StyleSheet.create({
  container: { flex: 1, justifyContent: 'center', padding: 28 },
  brand: { fontWeight: '800', fontSize: 14, letterSpacing: 2 },
  title: { marginTop: 14, fontWeight: '700', fontSize: 28, lineHeight: 35 },
  body: { marginTop: 18, fontSize: 15, lineHeight: 22 },
  button: { marginTop: 30, alignSelf: 'flex-start', borderRadius: 8, paddingHorizontal: 18, paddingVertical: 13 },
  buttonText: { color: '#fff', fontWeight: '700' },
});
