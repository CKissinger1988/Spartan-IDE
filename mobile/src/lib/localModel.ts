// §69.5's "On-device model for local Q&A and summarization (v2)": a small
// quantized model (the same llama.cpp-family runtime the desktop's Ollama/
// LM Studio integrations already speak, §3.3/§57 — not a new inference
// stack) handles offline "explain this diff" entirely on-device. Explicit
// honest boundary, the same way §66 states the CPU renderer's: this
// interface is real and something screens can call today, but there is no
// actual model behind it.
//
// Why this one stayed an interface-only stub rather than a real
// integration, unlike everything else built in this repo: a working
// version needs (a) a llama.cpp-family native binding for React Native —
// a large native module requiring a custom dev client, same category of
// constraint as voice capture, but heavier; (b) an actual quantized model
// file (hundreds of MB to a few GB) downloaded and stored on a real
// device; (c) real inference hardware to run it and observe output. None
// of the three is reachable in this environment — no device, no way to
// meaningfully fetch/store a multi-GB file as part of "the app," and no
// way to run or verify inference even if one were bundled. Faking a
// "working" client here would violate this project's own core rule: don't
// claim something works without running it.
//
// PrivacyScoped routing (§3.5, desktop spec) is the contract this
// interface's real implementation must respect once one exists: a repo
// marked local-only on desktop must stay local-only here too, so its
// artifact text never transits a cloud model from the phone either. The
// stub can't violate that because it never calls anything remote — a
// property worth keeping true of the real implementation, not just this
// placeholder.

export interface LocalModelClient {
  isAvailable(): Promise<boolean>;
  ask(question: string, context: string): Promise<string>;
}

class UnavailableLocalModelClient implements LocalModelClient {
  async isAvailable(): Promise<boolean> {
    return false;
  }

  async ask(): Promise<string> {
    throw new Error(
      'No on-device model is wired up in this build. See src/lib/localModel.ts for exactly ' +
        'what real integration this stands in for and why it is not built yet.'
    );
  }
}

// Swap this for a real client once a llama.cpp-family binding, a model
// file, and a device to run it on all actually exist together.
export const localModelClient: LocalModelClient = new UnavailableLocalModelClient();
