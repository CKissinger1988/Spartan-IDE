import React, { useEffect, useMemo, useRef, useState } from "react";
import { DEVICE_PROFILES, type DeviceProfile } from "./DevicePreview";

type SourceTab = "html" | "css" | "js";
const RESPONSIVE_BREAKPOINTS = [
  { id: "mobile", label: "Mobile ≤ 600px", maxWidth: 600 },
  { id: "tablet", label: "Tablet ≤ 900px", maxWidth: 900 },
  { id: "desktop", label: "Desktop ≥ 901px", maxWidth: 99999 },
] as const;

interface WebTemplate {
  id: string;
  label: string;
  description: string;
  html: string;
  css: string;
  js: string;
}

const WEB_TEMPLATES: WebTemplate[] = [
  {
    id: "starter",
    label: "Vanilla starter",
    description: "A small accessible HTML, CSS, and JavaScript app.",
    html: `<main class="app-shell">
  <p class="eyebrow">Spartan Web Studio</p>
  <h1>Build something great.</h1>
  <p class="lede">Edit the source, run it in an isolated preview, and check the console as you go.</p>
  <button id="action">Test interaction</button>
  <p id="status" role="status">Ready.</p>
</main>`,
    css: `:root { font-family: system-ui, sans-serif; color: #e8eef5; background: #111820; }
* { box-sizing: border-box; }
body { margin: 0; min-height: 100vh; display: grid; place-items: center; }
.app-shell { width: min(620px, 90vw); padding: 3rem; border: 1px solid #334454; border-radius: 24px; background: #182431; box-shadow: 0 24px 80px #0008; }
.eyebrow { color: #58d6a4; font-weight: 700; letter-spacing: .12em; text-transform: uppercase; }
h1 { margin: 0; font-size: clamp(2rem, 8vw, 4.5rem); line-height: .95; }
.lede { color: #a9b6c3; line-height: 1.6; }
button { padding: .8rem 1.1rem; color: #08120f; border: 0; border-radius: 10px; background: #58d6a4; font-weight: 700; cursor: pointer; }
#status { color: #a9b6c3; }`,
    js: `const button = document.querySelector("#action");
const status = document.querySelector("#status");
button?.addEventListener("click", () => {
  status.textContent = "Interaction received at " + new Date().toLocaleTimeString();
  console.info("Button interaction received");
});
console.log("Web Studio preview loaded");`,
  },
  {
    id: "landing",
    label: "Responsive landing page",
    description: "Hero, feature cards, and responsive layout primitives.",
    html: `<header class="nav"><strong>Northstar</strong><nav><a href="#features">Features</a><a href="#contact">Contact</a></nav></header>
<main><section class="hero"><div><p class="eyebrow">Ship with confidence</p><h1>Your next launch starts here.</h1><p>Turn a strong idea into a polished experience with a focused workflow.</p><a class="cta" href="#features">Explore the toolkit</a></div><div class="orb" aria-hidden="true"></div></section>
<section id="features" class="features"><article><span>01</span><h2>Compose</h2><p>Keep structure, style, and behavior easy to understand.</p></article><article><span>02</span><h2>Preview</h2><p>Verify responsive behavior at the viewport sizes that matter.</p></article><article><span>03</span><h2>Release</h2><p>Export a clean, portable bundle when the work is ready.</p></article></section></main>`,
    css: `:root { font-family: Inter, system-ui, sans-serif; color: #17212b; background: #f5f7fa; }
* { box-sizing: border-box; } body { margin: 0; } .nav { display: flex; justify-content: space-between; padding: 1.5rem 6vw; } nav { display: flex; gap: 1rem; } a { color: inherit; text-decoration: none; } .hero { display: grid; grid-template-columns: 1.2fr .8fr; gap: 4rem; align-items: center; max-width: 1100px; min-height: 70vh; margin: auto; padding: 5rem 6vw; } .eyebrow { color: #d65f38; font-weight: 800; text-transform: uppercase; letter-spacing: .14em; } h1 { max-width: 700px; margin: .4rem 0 1rem; font-size: clamp(3rem, 8vw, 7rem); line-height: .9; } .hero p:not(.eyebrow) { max-width: 520px; color: #536273; font-size: 1.2rem; line-height: 1.6; } .cta { display: inline-block; margin-top: 1rem; padding: .9rem 1.2rem; border-radius: 999px; color: white; background: #17212b; } .orb { aspect-ratio: 1; border-radius: 42% 58% 60% 40%; background: linear-gradient(135deg, #f7b267, #d65f38 55%, #5e3b8a); } .features { display: grid; grid-template-columns: repeat(3, 1fr); gap: 1rem; max-width: 1100px; margin: auto; padding: 2rem 6vw 5rem; } article { padding: 1.5rem; border: 1px solid #dce2e8; border-radius: 18px; background: white; } article span { color: #d65f38; font-weight: 800; } article p { color: #536273; line-height: 1.5; } @media (max-width: 700px) { .hero { grid-template-columns: 1fr; padding-top: 3rem; } .orb { max-width: 260px; } .features { grid-template-columns: 1fr; } }`,
    js: `console.log("Landing page template loaded");`,
  },
  {
    id: "dashboard",
    label: "Dashboard",
    description: "A compact data dashboard with responsive cards.",
    html: `<main class="dashboard"><header><div><p class="muted">Workspace overview</p><h1>Good morning, builder.</h1></div><button id="refresh">Refresh data</button></header><section class="stats"><article><span>Revenue</span><strong>$48,290</strong><small>+12.4% this month</small></article><article><span>Active users</span><strong>8,942</strong><small>+8.1% this month</small></article><article><span>Conversion</span><strong>6.8%</strong><small>+1.2% this month</small></article></section><section class="panel"><h2>Activity</h2><div class="chart"><i style="height: 35%"></i><i style="height: 60%"></i><i style="height: 45%"></i><i style="height: 78%"></i><i style="height: 68%"></i><i style="height: 92%"></i></div></section></main>`,
    css: `:root { font-family: system-ui, sans-serif; color: #e5e7eb; background: #0d1117; } * { box-sizing: border-box; } body { margin: 0; } .dashboard { max-width: 1000px; margin: auto; padding: 2rem; } header { display: flex; justify-content: space-between; gap: 1rem; align-items: end; } h1 { margin: .25rem 0 2rem; font-size: clamp(1.6rem, 5vw, 3rem); } .muted, small { color: #8b98a8; } button { padding: .7rem 1rem; border: 1px solid #314155; border-radius: 9px; color: #e5e7eb; background: #182231; cursor: pointer; } .stats { display: grid; grid-template-columns: repeat(3, 1fr); gap: 1rem; } article, .panel { padding: 1.3rem; border: 1px solid #263448; border-radius: 14px; background: #151d29; } article span, article small { display: block; } article strong { display: block; margin: .6rem 0; font-size: 2rem; } article small { color: #56d39a; } .panel { margin-top: 1rem; } .chart { height: 220px; display: flex; align-items: end; gap: 1rem; padding: 1rem 0 0; } .chart i { flex: 1; min-height: 8px; border-radius: 6px 6px 0 0; background: linear-gradient(#56d39a, #277d68); } @media (max-width: 650px) { .stats { grid-template-columns: 1fr; } header { align-items: start; flex-direction: column; } }`,
    js: `document.querySelector("#refresh")?.addEventListener("click", () => console.info("Dashboard refreshed"));
console.log("Dashboard template loaded");`,
  },
];

export interface WebSources { html: string; css: string; js: string }
interface SavedSources extends WebSources { templateId: string }

interface WebDevelopmentSuiteProps {
  initialSources?: WebSources;
  onSourcesChange?: (sources: WebSources) => void;
  persistSources?: boolean;
}

interface SelectedElement {
  selector: string;
  tagName: string;
  text: string;
  color: string;
  backgroundColor: string;
  fontSize: string;
}

function savedSources(): SavedSources | null {
  try {
    const value = JSON.parse(localStorage.getItem("spartan.web-suite.sources") ?? "null");
    return value && typeof value.html === "string" && typeof value.css === "string" && typeof value.js === "string" ? value : null;
  } catch { return null; }
}

function downloadFile(name: string, content: string, type: string): void {
  const url = URL.createObjectURL(new Blob([content], { type }));
  const link = document.createElement("a"); link.href = url; link.download = name; link.click();
  window.setTimeout(() => URL.revokeObjectURL(url), 1000);
}

function standaloneHtml(html: string, css: string, js: string): string {
  return `<!doctype html>\n<html lang="en">\n<head>\n  <meta charset="utf-8">\n  <meta name="viewport" content="width=device-width, initial-scale=1">\n  <style>${css}</style>\n</head>\n<body>\n${html}\n<script>${js.replace(/<\/script/gi, "<\\/script")}<\/script>\n</body>\n</html>\n`;
}

function projectHtml(html: string): string {
  return `<!doctype html>\n<html lang="en">\n<head>\n  <meta charset="utf-8">\n  <meta name="viewport" content="width=device-width, initial-scale=1">\n  <link rel="stylesheet" href="style.css">\n</head>\n<body>\n${html}\n<script src="script.js"><\/script>\n</body>\n</html>\n`;
}

function orientedSize(profile: DeviceProfile, landscape: boolean): [number, number] {
  return landscape ? [profile.height, profile.width] : [profile.width, profile.height];
}

export default function WebDevelopmentSuite({ initialSources, onSourcesChange, persistSources = true }: WebDevelopmentSuiteProps = {}): React.ReactElement {
  const saved = persistSources ? savedSources() : null;
  const initial = initialSources ?? saved ?? WEB_TEMPLATES[0];
  const [templateId, setTemplateId] = useState(saved?.templateId ?? WEB_TEMPLATES[0].id);
  const [html, setHtml] = useState(initial.html);
  const [css, setCss] = useState(initial.css);
  const [js, setJs] = useState(initial.js);
  const [sourceTab, setSourceTab] = useState<SourceTab>("html");
  const [profileId, setProfileId] = useState(DEVICE_PROFILES[0].id);
  const [landscape, setLandscape] = useState(false);
  const [zoom, setZoom] = useState(65);
  const [consoleLines, setConsoleLines] = useState<string[]>([]);
  const [previewVersion, setPreviewVersion] = useState(0);
  const sourcesInitialized = useRef(false);
  const [selectedElement, setSelectedElement] = useState<SelectedElement | null>(null);
  const [textDraft, setTextDraft] = useState("");
  const [styleDraft, setStyleDraft] = useState({ color: "", backgroundColor: "", fontSize: "" });
  const [responsiveBreakpoint, setResponsiveBreakpoint] = useState<(typeof RESPONSIVE_BREAKPOINTS)[number]["id"]>("mobile");
  const [responsiveDraft, setResponsiveDraft] = useState({ fontSize: "", padding: "", display: "" });
  const [draggedBlock, setDraggedBlock] = useState<"hero" | "cards" | "form" | null>(null);
  const profile = DEVICE_PROFILES.find((item) => item.id === profileId) ?? DEVICE_PROFILES[0];
  const [width, height] = orientedSize(profile, landscape);
  const source = sourceTab === "html" ? html : sourceTab === "css" ? css : js;
  const setSource = sourceTab === "html" ? setHtml : sourceTab === "css" ? setCss : setJs;
  const template = WEB_TEMPLATES.find((item) => item.id === templateId) ?? WEB_TEMPLATES[0];
  const previewDocument = useMemo(() => {
    const bridge = `<script>(function(){function send(level,args){parent.postMessage({source:"spartan-web-suite",level:level,args:Array.from(args).map(function(item){try{return typeof item === "string" ? item : JSON.stringify(item)}catch(_){return String(item)}})},"*")}function selector(el){if(el.id)return "#"+CSS.escape(el.id);var path=[];while(el&&el.nodeType===1&&el!==document.body){var part=el.tagName.toLowerCase();if(el.classList.length)part+="."+CSS.escape(el.classList[0]);var siblings=el.parentElement?Array.from(el.parentElement.children).filter(function(item){return item.tagName===el.tagName}):[];if(siblings.length>1)part+=":nth-of-type("+(siblings.indexOf(el)+1)+")";path.unshift(part);el=el.parentElement}return path.join(" ")}document.addEventListener("click",function(event){var el=event.target.closest("body *");if(!el)return;event.preventDefault();event.stopPropagation();document.querySelectorAll(".spartan-selected").forEach(function(item){item.classList.remove("spartan-selected")});el.classList.add("spartan-selected");var style=getComputedStyle(el);parent.postMessage({source:"spartan-web-suite",type:"select",selector:selector(el),tagName:el.tagName.toLowerCase(),text:(el.innerText||"").slice(0,500),color:style.color,backgroundColor:style.backgroundColor,fontSize:style.fontSize},"*")},true);var style=document.createElement("style");style.textContent=".spartan-selected{outline:2px solid #58d6a4!important;outline-offset:3px!important;cursor:crosshair!important}";document.head.appendChild(style);["log","info","warn","error"].forEach(function(level){var original=console[level];console[level]=function(){original.apply(console,arguments);send(level,arguments)}});window.addEventListener("error",function(event){send("error",[event.message])});window.addEventListener("unhandledrejection",function(event){send("error",[String(event.reason)])})})()<\/script>`;
    return `<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><style>${css}</style></head><body>${html}${bridge}<script>${js.replace(/<\/script/gi, "<\\/script")}<\/script></body></html>`;
  }, [css, html, js]);
  const frameStyle = useMemo(() => ({ "--web-width": `${width}px`, "--web-height": `${height}px`, "--web-zoom": zoom / 100 } as React.CSSProperties), [height, width, zoom]);

  useEffect(() => {
    const sources = { html, css, js };
    if (persistSources) localStorage.setItem("spartan.web-suite.sources", JSON.stringify({ ...sources, templateId }));
    if (sourcesInitialized.current) onSourcesChange?.(sources);
    sourcesInitialized.current = true;
  }, [css, html, js, onSourcesChange, persistSources, templateId]);

  useEffect(() => {
    const receive = (event: MessageEvent) => {
      if (event.data?.source !== "spartan-web-suite") return;
      if (event.data.type === "select") {
        const next: SelectedElement = {
          selector: String(event.data.selector ?? ""),
          tagName: String(event.data.tagName ?? "element"),
          text: String(event.data.text ?? ""),
          color: String(event.data.color ?? ""),
          backgroundColor: String(event.data.backgroundColor ?? ""),
          fontSize: String(event.data.fontSize ?? ""),
        };
        setSelectedElement(next);
        setTextDraft(next.text);
        setStyleDraft({ color: next.color, backgroundColor: next.backgroundColor, fontSize: next.fontSize });
        setResponsiveDraft({ fontSize: "", padding: "", display: "" });
        return;
      }
      const args = Array.isArray(event.data.args) ? event.data.args : [];
      setConsoleLines((lines) => [...lines, `[${event.data.level}] ${args.join(" ")}`].slice(-200));
    };
    window.addEventListener("message", receive);
    return () => window.removeEventListener("message", receive);
  }, []);

  const applyTemplate = () => {
    setHtml(template.html); setCss(template.css); setJs(template.js); setSelectedElement(null); setConsoleLines([]); setPreviewVersion((value) => value + 1);
  };

  const updateSelectedText = () => {
    if (!selectedElement) return;
    const parser = new DOMParser();
    const documentFragment = parser.parseFromString(`<body>${html}</body>`, "text/html");
    const target = documentFragment.body.querySelector(selectedElement.selector);
    if (!target) return;
    target.textContent = textDraft;
    setHtml(documentFragment.body.innerHTML);
    setPreviewVersion((value) => value + 1);
  };

  const updateSelectedStyles = () => {
    if (!selectedElement) return;
    const declarations = [
      ["color", styleDraft.color],
      ["background-color", styleDraft.backgroundColor],
      ["font-size", styleDraft.fontSize],
    ].filter(([, value]) => value.trim());
    if (!declarations.length) return;
    const rule = `${selectedElement.selector} { ${declarations.map(([property, value]) => `${property}: ${value}`).join("; ")} }`;
    setCss((value) => `${value}\n\n/* Spartan Web Studio inspector */\n${rule}`);
    setPreviewVersion((value) => value + 1);
  };

  const updateResponsiveStyles = () => {
    if (!selectedElement) return;
    const breakpoint = RESPONSIVE_BREAKPOINTS.find((item) => item.id === responsiveBreakpoint) ?? RESPONSIVE_BREAKPOINTS[0];
    const declarations = [
      ["font-size", responsiveDraft.fontSize],
      ["padding", responsiveDraft.padding],
      ["display", responsiveDraft.display],
    ].filter(([, value]) => value.trim());
    if (!declarations.length) return;
    const rule = `${selectedElement.selector} { ${declarations.map(([property, value]) => `${property}: ${value}`).join("; ")} }`;
    const media = breakpoint.id === "desktop"
      ? `@media (min-width: 901px) { ${rule} }`
      : `@media (max-width: ${breakpoint.maxWidth}px) { ${rule} }`;
    setCss((value) => `${value}\n\n/* Spartan responsive element override · ${breakpoint.label} */\n${media}`);
    setPreviewVersion((value) => value + 1);
  };

  const exportWebsitePackage = () => {
    downloadFile("index.html", projectHtml(html), "text/html");
    downloadFile("style.css", css, "text/css");
    downloadFile("script.js", js, "text/javascript");
    downloadFile("spartan-site.json", JSON.stringify({ format: "spartan-web-site", version: 1, html, css, js }, null, 2), "application/json");
  };

  const insertBlock = (kind: "hero" | "cards" | "form") => {
    const blocks = {
      hero: `<section class="spartan-block hero-block"><p class="eyebrow">New section</p><h2>Make your next idea visible.</h2><p>Replace this copy with your own message and style it from the inspector.</p><a class="cta" href="#">Learn more</a></section>`,
      cards: `<section class="spartan-block card-block"><article><h3>Feature one</h3><p>Explain the first benefit.</p></article><article><h3>Feature two</h3><p>Explain the second benefit.</p></article><article><h3>Feature three</h3><p>Explain the third benefit.</p></article></section>`,
      form: `<form class="spartan-block form-block"><label>Name<input name="name" placeholder="Your name"></label><label>Email<input name="email" type="email" placeholder="you@example.com"></label><button type="submit">Send message</button></form>`,
    }[kind];
    setHtml((value) => `${value}\n${blocks}`);
    setPreviewVersion((value) => value + 1);
  };

  const handleBlockDrop = (event: React.DragEvent) => {
    event.preventDefault();
    if (draggedBlock) insertBlock(draggedBlock);
    setDraggedBlock(null);
  };

  return <section className="web-suite" aria-label="Web development suite">
    <div className="web-suite-toolbar">
      <label className="device-preview-field">Template<select value={templateId} onChange={(event) => setTemplateId(event.target.value)}>{WEB_TEMPLATES.map((item) => <option key={item.id} value={item.id}>{item.label}</option>)}</select></label>
      <span className="web-suite-template-note">{template.description}</span>
      <button className="toolbar-btn" onClick={applyTemplate}>Apply template</button>
      <button className="toolbar-btn toolbar-btn-primary" onClick={() => { setConsoleLines([]); setPreviewVersion((value) => value + 1); }}>Run preview</button>
      <button className="toolbar-btn" draggable onDragStart={() => setDraggedBlock("hero")} onDragEnd={() => setDraggedBlock(null)} onClick={() => insertBlock("hero")}>+ Hero block</button>
      <button className="toolbar-btn" draggable onDragStart={() => setDraggedBlock("cards")} onDragEnd={() => setDraggedBlock(null)} onClick={() => insertBlock("cards")}>+ Cards block</button>
      <button className="toolbar-btn" draggable onDragStart={() => setDraggedBlock("form")} onDragEnd={() => setDraggedBlock(null)} onClick={() => insertBlock("form")}>+ Form block</button>
      <button className="toolbar-btn" onClick={() => downloadFile("index.html", standaloneHtml(html, css, js), "text/html")}>Export HTML</button>
    </div>
    <div className="web-suite-body">
      <div className="web-suite-source">
        <div className="web-suite-tabs">{(["html", "css", "js"] as SourceTab[]).map((tab) => <button key={tab} className={`web-suite-tab ${sourceTab === tab ? "web-suite-tab-active" : ""}`} onClick={() => setSourceTab(tab)}>{tab.toUpperCase()}</button>)}</div>
        <textarea className="web-suite-editor mono" aria-label={`${sourceTab.toUpperCase()} source`} value={source} onChange={(event) => setSource(event.target.value)} spellCheck={false} />
        <div className="web-suite-source-footer mono">Edits are saved locally · {source.split("\n").length} lines · {source.length} chars</div>
      </div>
      <div className="web-suite-preview-column">
        <div className="web-suite-preview-toolbar">
          <label>Viewport<select value={profileId} onChange={(event) => setProfileId(event.target.value)}>{DEVICE_PROFILES.map((item) => <option key={item.id} value={item.id}>{item.label}</option>)}</select></label>
          <button className="toolbar-btn" onClick={() => setLandscape((value) => !value)}>{landscape ? "Portrait" : "Landscape"}</button>
          <label className="web-suite-zoom">Zoom {zoom}%<input type="range" min="25" max="100" step="5" value={zoom} onChange={(event) => setZoom(Number(event.target.value))} /></label>
          <span className="web-suite-dimensions mono">{width} × {height} · DPR {profile.pixelRatio}</span>
        </div>
        <div className={`web-suite-stage ${draggedBlock ? "web-suite-stage-drop-active" : ""}`} onDragOver={(event) => event.preventDefault()} onDrop={handleBlockDrop}>
          <div className={`web-suite-frame web-suite-frame-${profile.platform}`} style={frameStyle}>
            <iframe key={previewVersion} title="Web Studio live preview" srcDoc={previewDocument} sandbox="allow-forms allow-modals allow-popups allow-scripts" />
          </div>
          {draggedBlock && <div className="web-suite-drop-hint mono">Drop to insert {draggedBlock} block</div>}
        </div>
        <div className="web-suite-inspector">
          <div className="web-suite-console-header"><span>Inspector</span><span className="mono">{selectedElement ? `${selectedElement.tagName} · ${selectedElement.selector}` : "Click an element in the preview"}</span></div>
          {selectedElement && <>
            <label>Text<input value={textDraft} onChange={(event) => setTextDraft(event.target.value)} /></label>
            <label>Text color<input value={styleDraft.color} onChange={(event) => setStyleDraft((value) => ({ ...value, color: event.target.value }))} /></label>
            <label>Background<input value={styleDraft.backgroundColor} onChange={(event) => setStyleDraft((value) => ({ ...value, backgroundColor: event.target.value }))} /></label>
            <label>Font size<input value={styleDraft.fontSize} onChange={(event) => setStyleDraft((value) => ({ ...value, fontSize: event.target.value }))} /></label>
            <button className="toolbar-btn toolbar-btn-primary" onClick={() => { updateSelectedText(); updateSelectedStyles(); }}>Apply inspector changes</button>
            <label>Breakpoint<select value={responsiveBreakpoint} onChange={(event) => setResponsiveBreakpoint(event.target.value as typeof responsiveBreakpoint)}>{RESPONSIVE_BREAKPOINTS.map((item) => <option key={item.id} value={item.id}>{item.label}</option>)}</select></label>
            <label>Responsive font<input value={responsiveDraft.fontSize} placeholder="e.g. 2rem" onChange={(event) => setResponsiveDraft((value) => ({ ...value, fontSize: event.target.value }))} /></label>
            <label>Responsive padding<input value={responsiveDraft.padding} placeholder="e.g. 1rem" onChange={(event) => setResponsiveDraft((value) => ({ ...value, padding: event.target.value }))} /></label>
            <label>Responsive display<input value={responsiveDraft.display} placeholder="e.g. none" onChange={(event) => setResponsiveDraft((value) => ({ ...value, display: event.target.value }))} /></label>
            <button className="toolbar-btn" onClick={updateResponsiveStyles}>Apply responsive override</button>
          </>}
        </div>
        <div className="web-suite-console"><div className="web-suite-console-header"><span>Console</span><button className="toolbar-btn" onClick={() => setConsoleLines([])}>Clear</button></div><pre className="mono">{consoleLines.length ? consoleLines.join("\n") : "Run preview to see console output."}</pre></div>
      </div>
    </div>
    <div className="web-suite-footer"><span>Sandboxed live preview · source changes are saved locally</span><button className="toolbar-btn" onClick={exportWebsitePackage}>Export website package</button></div>
  </section>;
}
