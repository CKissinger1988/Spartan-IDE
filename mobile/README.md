# Spartan Mobile IDE

Companion mobile app for Spartan IDE — this `mobile/` subdirectory of the same repo, on the same
branch. A separate platform, stack, and release cycle from the desktop IDE, but not a separate
repository (it briefly was one; that changed by explicit decision — its real commit history was
preserved via `git subtree` rather than squashed). The design this was built from is
`../docs/architecture-spec.md` §69 (§69.1–§69.7): what this app is, what it deliberately is not,
and why.

## What this actually is (and is not)

A **companion app**, not a ported IDE. No code editing, no debugger, no terminal, no Developer
Mode — a phone is the wrong form factor for any of that, and §69.1 says so explicitly rather than
leaving it an unstated gap.

**v1 scope — all built:**

- Inbox / Agent Manager mirror — session threads across workspaces, same running/review/done
  model as desktop, with search (title/workspace) and status filtering
- Chat with Leo, same conversation history as desktop/CLI, with a real message composer (no fake
  Leo replies — see Current status below)
- Approve/Reject on Diff Cards and Implementation Plans, gated behind biometric approval (§69.5)
- Read-only, **actually** syntax-highlighted diff viewing for review context (`src/lib/diffHighlight.ts`
  parses unified-diff lines into add/remove/hunk/context, colored accordingly)
- Artifact commenting, including per-file comments (not just whole-artifact ones)
- Push-notification permission request and settings toggle (client-side only)
- Offline-first review queue: an Approve/Reject made with no connection is queued on-device with
  the artifact state the reviewer saw, instead of silently failing or pretending to succeed
- Network-aware connectivity check (Wi-Fi vs. cellular vs. offline) shown in Settings
- Edge-cached repo context: the Inbox thread list and any viewed artifact are cached on-device
  and fall back to the cached copy if the live source comes up empty
- Notification-surface actions: Approve/Reject buttons directly on the notification for
  low-stakes artifacts; destructive-class artifacts only open the app to the gated review screen
- Decision History (§69.7) — a local, persistent log of every Approve/Reject decision made on the
  device, reachable from Settings; a local audit log, not a synced one

**v2 scope — built where genuinely possible, honestly stubbed where not:**

- Camera capture into artifacts — real, Expo-Go-compatible (`expo-image-picker`): take or pick a
  photo, attach it to a session. OCR text extraction is *not* implemented — see below.
- Voice-to-task capture — real code against `expo-speech-recognition`'s documented API, but this
  is a third-party native module that needs a custom dev client (not Expo Go) to actually run —
  see below for exactly what that means for confidence in this one.
- On-device model Q&A — deliberately **not** a real integration. See `src/lib/localModel.ts` for
  why (needs a llama.cpp-family native binding, a multi-GB model file, and real inference
  hardware — none reachable here) and what's built instead: a real interface + UI affordance
  (`ArtifactReviewScreen`'s "Ask about this diff" button) wired to a stub that says so honestly
  rather than faking an answer.

## Stack

React Native via Expo (SDK 57), TypeScript. §69.2 named Flutter and React Native as the two real
candidates and deliberately deferred the choice "until Tier assignment makes this actionable" —
React Native was picked when that became actionable.

## Current status — read this before assuming more is done than is

**Real code, not a mockup**: `App.tsx` boots a real `NavigationContainer` with six real,
type-checked screens (`src/screens/`) wired to real navigation params (`src/navigation/`) — the
original five plus `DecisionHistoryScreen` (§69.7). Verified after every change by actually
running:

```bash
npx tsc --noEmit          # clean
npx expo export --platform android   # real Metro bundle, 931 modules, succeeds
npm test                  # jest-expo + @testing-library/react-native, 93 tests, real assertions
```

**Real Jest coverage exists for both the business-logic layer and the screens themselves.**
`src/lib/__tests__/` and `src/data/__tests__/` cover `offlineQueue`, `decisionActions`,
`biometricGate`, `edgeCache`, `network`, `notificationActions`, `localTaskStore`, `decisionHistory`,
`diffHighlight`, and the `localModel` stub, using the first-party jest mocks for AsyncStorage/
NetInfo and hand-written mocks for the Expo modules that ship none. `src/screens/__tests__/`
(§69.7) covers all six screens as rendered `@testing-library/react-native` components — render
output, user interaction (typing, pressing, filtering), and the outcome of mocked async calls.
Every test suite across both layers was spot-checked by deliberately breaking the real behavior
it claims to cover and confirming the test actually fails, not just inspected for plausibility.

One dependency-version quirk worth knowing if you add more screen tests: this repo's installed
`@testing-library/react-native` 14.x, paired with React 19's concurrent renderer, makes `render()`
and `fireEvent.*` asynchronous — they must be `await`ed, or queries silently fail or events never
flush before assertions run. This isn't documented prominently in most RNTL examples online.

**Backed entirely by mock data** (`src/data/mockData.ts`) — there is no session-store backend to
talk to yet. Every screen reads local placeholder data; nothing here syncs, persists, or sends a
decision anywhere.

**Not run on a device, simulator, or emulator** — this environment has no Android/iOS
device/emulator, matching the same no-GPU/no-display constraint documented for the desktop repo.
`expo export` proves the JS bundle builds correctly; it proves nothing about how the UI actually
renders or behaves on a real screen, and specifically does **not** prove the Face ID/fingerprint
prompt, notification permissions/actions, camera capture, or voice recognition actually work on
real hardware — all are only exercised against their documented Expo APIs here, never an actual
OS-level prompt, tap, or microphone. Don't take a clean export as a substitute for running the
app.

**Three different confidence levels for "will this work on a real device," stated explicitly
rather than flattened into one claim:**

1. **High confidence** — `expo-notifications`, `expo-local-authentication`, `expo-secure-store`,
   `@react-native-async-storage/async-storage`, `@react-native-community/netinfo`,
   `expo-image-picker`: first-party or Expo-Go-compatible modules, part of the managed workflow
   Expo Go itself ships. Untested here, but there's real reason to expect them to work day one.
2. **Unverifiable beyond JS bundling** — `expo-speech-recognition`: a third-party native module
   requiring a custom dev client (EAS Build or a local Xcode/Android Studio prebuild) to run at
   all. `expo export` succeeding proves only that its JS/TS glue resolves — it says nothing about
   the native side, which was never built here.
3. **Deliberately not integrated** — on-device model Q&A (`src/lib/localModel.ts`). A real
   version needs a llama.cpp-family binding, a downloaded model file, and inference hardware;
   none of the three exist in this environment, so this stayed an honest stub rather than a
   faked integration.

**The edge cache is honestly split, not uniformly "encrypted"** (`src/lib/edgeCache.ts`):
SecureStore (Keychain/Keystore-backed) holds only a small manifest, because it has a real
per-key size ceiling (~2KB on Android) that a full diff patch can exceed; the actual cached
diff/session content lives in AsyncStorage, which is app-sandboxed but not hardware-encryption-
backed.

**What's still not built**: a real push/sync backend (nothing in this repo can send or receive
anything remote — every "queued," "cached," or "local only" note in the code means exactly that),
on-device OCR for captured images (`ImageAttachment.ocrText` is always `null`; extracting it
needs another native module in the same category as voice recognition), and an actual local
model behind the Q&A affordance. Building a real backend is a different, much larger project than
"the mobile companion app," and is not attempted here.

## Running it

```bash
npm install
npm start          # then press a/i/w for Android/iOS/web, or scan the QR code in Expo Go
```

`npm run android` / `npm run ios` need a real device or emulator/simulator, neither of which was
available when this was built — untested beyond `tsc`/`expo export` for that reason. Voice
capture additionally needs a custom dev client; Expo Go alone won't load its native module.
