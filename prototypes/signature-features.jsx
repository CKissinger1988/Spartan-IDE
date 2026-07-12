import React, { useState, useEffect, useRef } from "react";
import {
  History,
  ShieldCheck,
  Terminal,
  Check,
  ChevronDown,
  X,
  GitBranch,
  Cpu,
  TestTube2,
  Sparkles,
  ArrowRight,
  Info,
} from "lucide-react";

const C = {
  bg: "#0B0B0D",
  s1: "#141416",
  s2: "#1B1B1E",
  s3: "#222226",
  border: "#28282C",
  borderLt: "#35353A",
  text: "#E9E7E4",
  textMid: "#A6A5A2",
  textDim: "#6E6D73",
  accent: "#2E7DFF", // §75.95 rebrand: blue, was rust/terracotta #C4432B
  accentBg: "rgba(46,125,255,0.14)",
  accentBorder: "rgba(46,125,255,0.4)",
  green: "#4E9E72",
  greenBg: "rgba(78,158,114,0.13)",
  amber: "#C99A3D",
  amberBg: "rgba(201,154,61,0.13)",
  red: "#B2453B",
  teal: "#3D9C93",
  tealBg: "rgba(61,156,147,0.13)",
};
const F_UI = "'Space Grotesk', ui-sans-serif, system-ui, sans-serif";
const F_MONO = "'IBM Plex Mono', ui-monospace, monospace";

/* ---------------- Timeline Scrubber ---------------- */
const VARIANTS = [
  {
    label: "A · Simple",
    latency: 340,
    tests: "12 / 12",
    note: "Naive recompute, easiest to read",
    code: [
      "pub fn get_price(sku: &str, catalog: &Catalog) -> Price {",
      "    // recomputes from scratch on every call",
      "    catalog.lookup(sku).apply_rules(&catalog.rules)",
      "}",
    ],
  },
  {
    label: "B · Cached",
    latency: 4,
    tests: "12 / 12",
    note: "HashMap cache, invalidated on rule change",
    code: [
      "pub fn get_price(sku: &str, catalog: &Catalog) -> Price {",
      "    if let Some(p) = CACHE.get(sku) { return *p; }",
      "    let price = catalog.lookup(sku).apply_rules(&catalog.rules);",
      "    CACHE.insert(sku.to_string(), price);",
      "    price",
      "}",
    ],
  },
  {
    label: "C · Async + Cache",
    latency: 2,
    tests: "11 / 12",
    note: "Fastest — but one concurrency test is flaky",
    code: [
      "pub async fn get_price(sku: &str, catalog: &Catalog) -> Price {",
      "    if let Some(p) = CACHE.get_async(sku).await { return p; }",
      "    let price = catalog.lookup(sku).apply_rules(&catalog.rules);",
      "    CACHE.insert_async(sku, price).await;",
      "    price",
      "}",
    ],
  },
];

function zoneFromScrub(v) {
  if (v < 33) return 0;
  if (v < 66) return 1;
  return 2;
}

function CodeLine({ text }) {
  const kw = ["pub", "fn", "async", "let", "if", "return", "await"];
  const parts = text.split(/(\s+)/);
  return (
    <div style={{ fontFamily: F_MONO, fontSize: 12.5, lineHeight: 1.7, color: C.text, whiteSpace: "pre" }}>
      {parts.map((p, i) =>
        kw.includes(p) ? (
          <span key={i} style={{ color: C.accent }}>
            {p}
          </span>
        ) : (
          <span key={i}>{p}</span>
        )
      )}
    </div>
  );
}

function AnimatedNumber({ value, suffix }) {
  const [display, setDisplay] = useState(value);
  const raf = useRef(null);
  useEffect(() => {
    const start = display;
    const delta = value - start;
    const t0 = performance.now();
    const dur = 260;
    function tick(now) {
      const p = Math.min(1, (now - t0) / dur);
      setDisplay(start + delta * (1 - Math.pow(1 - p, 3)));
      if (p < 1) raf.current = requestAnimationFrame(tick);
    }
    raf.current = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf.current);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [value]);
  return (
    <span>
      {display.toFixed(0)}
      {suffix}
    </span>
  );
}

function TimelineScrubber() {
  const [scrub, setScrub] = useState(50);
  const zone = zoneFromScrub(scrub);
  const v = VARIANTS[zone];

  return (
    <div className="fade-in-up">
      <div className="flex items-center gap-2 mb-1">
        <History size={15} color={C.accent} />
        <span style={{ fontFamily: F_UI, fontSize: 14, fontWeight: 600, color: C.text }}>Timeline Scrubber</span>
      </div>
      <p style={{ fontFamily: F_UI, fontSize: 12, color: C.textDim, marginBottom: 16, lineHeight: 1.6 }}>
        Three implementation attempts, generated in parallel sandboxes and run against the real test suite. Scrub
        between them — this isn't a static diff, it's cheap because a checkpoint clone measured at ~0.0002ms in Spike
        0.1 (§47.1) is what makes dragging through history feel free.
      </p>

      <div className="rounded-lg overflow-hidden" style={{ border: `1px solid ${C.border}`, background: C.s1 }}>
        <div
          className="flex items-center justify-between px-4 py-2.5"
          style={{ borderBottom: `1px solid ${C.border}`, background: C.s2 }}
        >
          <div className="flex items-center gap-2">
            <GitBranch size={12} color={C.textDim} />
            <span style={{ fontFamily: F_MONO, fontSize: 11.5, color: C.text }}>pricing.rs</span>
          </div>
          <span
            className="rounded px-2 py-0.5"
            style={{ background: C.accentBg, color: C.accent, fontFamily: F_MONO, fontSize: 10.5 }}
          >
            {v.label}
          </span>
        </div>

        <div className="px-4 py-4" style={{ minHeight: 168 }} key={zone}>
          <div className="fade-in-up">
            {v.code.map((l, i) => (
              <CodeLine key={i} text={l} />
            ))}
          </div>
        </div>

        <div
          className="flex items-center gap-5 px-4 py-2.5"
          style={{ borderTop: `1px solid ${C.border}`, background: C.s2 }}
        >
          <div className="flex items-center gap-1.5">
            <Cpu size={11} color={C.textDim} />
            <span style={{ fontFamily: F_MONO, fontSize: 11, color: C.text }}>
              <AnimatedNumber value={v.latency} suffix="ms" />
            </span>
          </div>
          <div className="flex items-center gap-1.5">
            <TestTube2 size={11} color={v.tests.startsWith("12") ? C.green : C.amber} />
            <span style={{ fontFamily: F_MONO, fontSize: 11, color: v.tests.startsWith("12") ? C.green : C.amber }}>
              {v.tests} passing
            </span>
          </div>
          <span style={{ fontFamily: F_UI, fontSize: 11, color: C.textDim, marginLeft: "auto" }}>{v.note}</span>
        </div>
      </div>

      <div className="mt-4 px-1">
        <input
          type="range"
          min={0}
          max={100}
          value={scrub}
          onChange={(e) => setScrub(Number(e.target.value))}
          className="w-full"
          style={{ accentColor: C.accent }}
        />
        <div className="flex justify-between mt-1">
          {VARIANTS.map((vv, i) => (
            <span key={i} style={{ fontFamily: F_MONO, fontSize: 10, color: i === zone ? C.accent : C.textDim }}>
              {vv.label}
            </span>
          ))}
        </div>
      </div>
    </div>
  );
}

/* ---------------- Trust Card ---------------- */
const GUARANTEES = [
  {
    title: "Model Integrity Guarantee",
    ref: "§36.4.5",
    short: "The badge you see is the model that actually ran your request.",
    detail:
      "Sourced directly from the ModelProvider that executed the call — never from a display-layer config that could drift from reality. This closes the exact gap behind a documented competitor CVE where the model dropdown showed one thing and silently routed to another.",
  },
  {
    title: "Single Writer Invariant",
    ref: "§36.4.1",
    short: "Every edit goes through one write lock, always.",
    detail:
      "Leo's edits, canvas edits, and manual keystrokes all route through the same rope-edit pipeline. A conflicting write surfaces as a reviewable Conflict artifact — it never silently overwrites the other.",
  },
  {
    title: "Path-Jailing",
    ref: "§36.4.6",
    short: "No tool call can ever resolve outside this project.",
    detail:
      "Canonicalized and hard-jailed to the project root at the sandbox layer — no ../ traversal, no symlink escape, regardless of what any model requests.",
  },
  {
    title: "Untrusted-Repo Quarantine",
    ref: "§36.4.2",
    short: "New repos get no auto-run and no secrets access until you say so.",
    detail:
      "This overrides even an 'autonomous' global setting. Opening an unfamiliar repo is always safe by default, not just usually.",
  },
  {
    title: "Transparent Pricing",
    ref: "§36.4.7",
    short: "Quota or pricing changes are announced before they take effect.",
    detail: "Never overnight, never silent. The cost dashboard reflects the actual current terms in real time.",
  },
  {
    title: "No Forced Updates",
    ref: "§18",
    short: "Pin your version. Choose your channel.",
    detail:
      "Stable, beta, or nightly — your choice, always reversible. This app will never split itself in two behind your back.",
  },
];

function TrustCard() {
  const [open, setOpen] = useState(null);
  return (
    <div className="fade-in-up">
      <div className="flex items-center gap-2 mb-1">
        <ShieldCheck size={15} color={C.accent} />
        <span style={{ fontFamily: F_UI, fontSize: 14, fontWeight: 600, color: C.text }}>Trust Card</span>
      </div>
      <p style={{ fontFamily: F_UI, fontSize: 12, color: C.textDim, marginBottom: 16, lineHeight: 1.6 }}>
        Shown on first launch. Not marketing copy — every line here is checked against the release-gate checklist in
        §36.5 before it ships.
      </p>
      <div className="space-y-2">
        {GUARANTEES.map((g, i) => {
          const isOpen = open === i;
          return (
            <div
              key={i}
              className="rounded-lg overflow-hidden cursor-pointer"
              style={{ border: `1px solid ${isOpen ? C.accentBorder : C.border}`, background: C.s1 }}
              onClick={() => setOpen(isOpen ? null : i)}
            >
              <div className="flex items-center gap-3 px-4 py-3">
                <div
                  className="rounded-full flex items-center justify-center flex-shrink-0"
                  style={{ width: 20, height: 20, background: C.greenBg }}
                >
                  <Check size={12} color={C.green} />
                </div>
                <div className="flex-1 min-w-0">
                  <div style={{ fontFamily: F_UI, fontSize: 12.5, fontWeight: 600, color: C.text }}>{g.title}</div>
                  <div style={{ fontFamily: F_UI, fontSize: 11.5, color: C.textMid, marginTop: 1 }}>{g.short}</div>
                </div>
                <span style={{ fontFamily: F_MONO, fontSize: 10, color: C.textDim, flexShrink: 0 }}>{g.ref}</span>
                <ChevronDown
                  size={14}
                  color={C.textDim}
                  className="flex-shrink-0"
                  style={{ transform: isOpen ? "rotate(180deg)" : "rotate(0)", transition: "transform 150ms ease" }}
                />
              </div>
              {isOpen && (
                <div className="px-4 pb-3.5 fade-in-up" style={{ paddingLeft: 51 }}>
                  <p style={{ fontFamily: F_UI, fontSize: 11.5, color: C.textMid, lineHeight: 1.6 }}>{g.detail}</p>
                  <span
                    className="inline-flex items-center gap-1 mt-2 rounded px-1.5 py-0.5"
                    style={{ background: C.tealBg, color: C.teal, fontFamily: F_MONO, fontSize: 10 }}
                  >
                    <ShieldCheck size={9} /> Verified by release gate §36.5
                  </span>
                </div>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}

/* ---------------- Session Handoff ---------------- */
function SessionHandoff() {
  const [show, setShow] = useState(false);
  const [replayKey, setReplayKey] = useState(0);

  useEffect(() => {
    const t = setTimeout(() => setShow(true), 500);
    return () => clearTimeout(t);
  }, [replayKey]);

  return (
    <div className="fade-in-up">
      <div className="flex items-center gap-2 mb-1">
        <Terminal size={15} color={C.accent} />
        <span style={{ fontFamily: F_UI, fontSize: 14, fontWeight: 600, color: C.text }}>One Leo, Felt</span>
      </div>
      <p style={{ fontFamily: F_UI, fontSize: 12, color: C.textDim, marginBottom: 16, lineHeight: 1.6 }}>
        §46 made the CLI and desktop share one session store architecturally. This is that fact, made visible the moment
        it matters — opening the desktop app after working from the terminal.
      </p>

      <div
        className="rounded-lg overflow-hidden relative"
        style={{ border: `1px solid ${C.border}`, background: C.s1, minHeight: 200 }}
      >
        <div
          className="flex items-center gap-2 px-3 py-2"
          style={{ borderBottom: `1px solid ${C.border}`, background: C.s2 }}
        >
          <div className="flex gap-1.5">
            <span className="rounded-full" style={{ width: 8, height: 8, background: C.s3 }} />
            <span className="rounded-full" style={{ width: 8, height: 8, background: C.s3 }} />
            <span className="rounded-full" style={{ width: 8, height: 8, background: C.s3 }} />
          </div>
          <span style={{ fontFamily: F_UI, fontSize: 11, color: C.textDim, marginLeft: 8 }}>Spartan — opening…</span>
        </div>

        <div className="p-3 space-y-2">
          <div className="rounded-md px-3 py-2 flex items-center gap-2" style={{ background: C.s2, opacity: 0.5 }}>
            <div className="rounded-full" style={{ width: 6, height: 6, background: C.green }} />
            <span style={{ fontFamily: F_UI, fontSize: 11.5, color: C.textMid }}>Refactor auth to JWT</span>
          </div>
          <div className="rounded-md px-3 py-2 flex items-center gap-2" style={{ background: C.s2, opacity: 0.5 }}>
            <div className="rounded-full" style={{ width: 6, height: 6, background: C.textDim }} />
            <span style={{ fontFamily: F_UI, fontSize: 11.5, color: C.textMid }}>Add Compose settings screen</span>
          </div>
        </div>

        {show && (
          <div
            className="absolute left-3 right-3 bottom-3 rounded-lg px-3.5 py-3 fade-in-up"
            style={{ background: C.s3, border: `1px solid ${C.accentBorder}`, boxShadow: "0 8px 24px rgba(0,0,0,0.4)" }}
          >
            <div className="flex items-start gap-2.5">
              <div
                className="rounded-full flex items-center justify-center flex-shrink-0 mt-0.5"
                style={{ width: 22, height: 22, background: C.accentBg }}
              >
                <Terminal size={12} color={C.accent} />
              </div>
              <div className="flex-1">
                <div style={{ fontFamily: F_UI, fontSize: 12, fontWeight: 600, color: C.text }}>
                  Picking up from your CLI session
                </div>
                <div style={{ fontFamily: F_MONO, fontSize: 10.5, color: C.textDim, marginTop: 2 }}>
                  14 min ago · 3 files changed · tests passing
                </div>
                <div className="flex items-center gap-2 mt-2.5">
                  <button
                    className="flex items-center gap-1 rounded-md px-2.5 py-1"
                    style={{ background: C.accent, color: "#fff", fontFamily: F_UI, fontSize: 11 }}
                  >
                    View in Agent View <ArrowRight size={11} />
                  </button>
                  <button onClick={() => setShow(false)} style={{ color: C.textDim }}>
                    <X size={13} />
                  </button>
                </div>
              </div>
            </div>
          </div>
        )}
      </div>

      <button
        onClick={() => {
          setShow(false);
          setReplayKey((k) => k + 1);
        }}
        className="mt-3 flex items-center gap-1.5 rounded-md px-3 py-1.5"
        style={{
          background: C.s2,
          border: `1px solid ${C.border}`,
          color: C.textMid,
          fontFamily: F_UI,
          fontSize: 11.5,
        }}
      >
        <Sparkles size={12} /> Replay the moment
      </button>
    </div>
  );
}

/* ---------------- Root ---------------- */
export default function SignatureFeatures() {
  const [tab, setTab] = useState("scrubber");
  const tabs = [
    { id: "scrubber", label: "Timeline Scrubber", icon: History },
    { id: "trust", label: "Trust Card", icon: ShieldCheck },
    { id: "handoff", label: "One Leo, Felt", icon: Terminal },
  ];

  return (
    <div className="min-h-screen w-full flex justify-center py-10 px-4" style={{ background: C.bg, fontFamily: F_UI }}>
      <style>{`
        @import url('https://fonts.googleapis.com/css2?family=Space+Grotesk:wght@400;500;600;700&family=IBM+Plex+Mono:wght@400;500;600&display=swap');
        @keyframes fadeInUp { from { opacity:0; transform: translateY(4px);} to { opacity:1; transform: translateY(0);} }
        .fade-in-up { animation: fadeInUp 220ms ease-out; }
        input[type=range] { -webkit-appearance: none; height: 4px; background: ${C.s3}; border-radius: 4px; }
        input[type=range]::-webkit-slider-thumb { -webkit-appearance: none; width: 14px; height: 14px; border-radius: 50%; background: ${C.accent}; cursor: pointer; margin-top: -5px; }
      `}</style>

      <div className="w-full max-w-2xl">
        <div className="flex items-center gap-2 mb-1">
          <Info size={13} color={C.textDim} />
          <span style={{ fontFamily: F_MONO, fontSize: 11, color: C.textDim }}>
            Spartan IDE — Signature Differentiators
          </span>
        </div>
        <h1 style={{ fontFamily: F_UI, fontWeight: 700, fontSize: 22, color: C.text, marginBottom: 20 }}>
          What makes this one of a kind
        </h1>

        <div
          className="flex gap-1 rounded-lg p-0.5 mb-6"
          style={{ background: C.s2, border: `1px solid ${C.border}`, width: "fit-content" }}
        >
          {tabs.map((t) => {
            const active = tab === t.id;
            const Icon = t.icon;
            return (
              <button
                key={t.id}
                onClick={() => setTab(t.id)}
                className="flex items-center gap-1.5 rounded-md px-3 py-1.5"
                style={{
                  fontFamily: F_UI,
                  fontSize: 12,
                  fontWeight: 500,
                  background: active ? C.accentBg : "transparent",
                  color: active ? C.accent : C.textMid,
                  border: `1px solid ${active ? C.accentBorder : "transparent"}`,
                }}
              >
                <Icon size={12} /> {t.label}
              </button>
            );
          })}
        </div>

        {tab === "scrubber" && <TimelineScrubber />}
        {tab === "trust" && <TrustCard />}
        {tab === "handoff" && <SessionHandoff />}
      </div>
    </div>
  );
}
