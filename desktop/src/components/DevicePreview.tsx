import React, { useMemo, useState } from "react";

export interface DeviceProfile {
  id: string;
  label: string;
  width: number;
  height: number;
  pixelRatio: number;
  platform: "ios" | "android" | "tablet" | "desktop";
}

export const DEVICE_PROFILES: DeviceProfile[] = [
  { id: "iphone-15", label: "iPhone 15", width: 393, height: 852, pixelRatio: 3, platform: "ios" },
  { id: "pixel-8", label: "Pixel 8", width: 412, height: 915, pixelRatio: 2.625, platform: "android" },
  { id: "ipad-air", label: "iPad Air", width: 820, height: 1180, pixelRatio: 2, platform: "tablet" },
  { id: "galaxy-tab", label: "Galaxy Tab", width: 800, height: 1280, pixelRatio: 2, platform: "tablet" },
  { id: "desktop-hd", label: "Desktop HD", width: 1440, height: 900, pixelRatio: 1, platform: "desktop" },
];

function orientedSize(profile: DeviceProfile, landscape: boolean): [number, number] {
  return landscape ? [profile.height, profile.width] : [profile.width, profile.height];
}

/** A real browser viewport emulator for responsive UI verification. It uses
 * an isolated iframe with the selected CSS viewport, rotation, zoom, safe-area
 * preview, and device metadata; it does not claim to emulate native hardware.
 */
export default function DevicePreview(): React.ReactElement {
  const [profileId, setProfileId] = useState(DEVICE_PROFILES[0].id);
  const [landscape, setLandscape] = useState(false);
  const [zoom, setZoom] = useState(75);
  const [url, setUrl] = useState("http://localhost:5173");
  const [source, setSource] = useState(url);
  const [showSafeArea, setShowSafeArea] = useState(true);
  const profile = DEVICE_PROFILES.find((item) => item.id === profileId) ?? DEVICE_PROFILES[0];
  const [width, height] = orientedSize(profile, landscape);
  const scale = zoom / 100;
  const frameStyle = useMemo(
    () => ({ "--device-width": `${width}px`, "--device-height": `${height}px`, "--device-scale": scale } as React.CSSProperties),
    [width, height, scale]
  );

  return (
    <section className="device-preview" aria-label="Device preview emulator">
      <div className="device-preview-toolbar">
        <label className="device-preview-field">
          Device
          <select value={profileId} onChange={(event) => setProfileId(event.target.value)}>
            {DEVICE_PROFILES.map((item) => (
              <option key={item.id} value={item.id}>{item.label}</option>
            ))}
          </select>
        </label>
        <button className="toolbar-btn" onClick={() => setLandscape((value) => !value)}>
          {landscape ? "Portrait" : "Landscape"}
        </button>
        <label className="device-preview-field device-preview-zoom">
          Zoom {zoom}%
          <input type="range" min="25" max="100" step="5" value={zoom} onChange={(event) => setZoom(Number(event.target.value))} />
        </label>
        <label className="device-preview-check">
          <input type="checkbox" checked={showSafeArea} onChange={(event) => setShowSafeArea(event.target.checked)} />
          Safe area
        </label>
        <span className="device-preview-meta mono">
          {width} × {height} CSS px · DPR {profile.pixelRatio}
        </span>
      </div>
      <form className="device-preview-url" onSubmit={(event) => { event.preventDefault(); setSource(url); }}>
        <input aria-label="Preview URL" value={url} onChange={(event) => setUrl(event.target.value)} placeholder="http://localhost:5173" />
        <button className="toolbar-btn toolbar-btn-primary" type="submit">Load</button>
        <span className="device-preview-platform mono">{profile.platform.toUpperCase()}</span>
      </form>
      <div className="device-preview-stage">
        <div className={`device-frame device-frame-${profile.platform} ${showSafeArea ? "device-frame-safe" : ""}`} style={frameStyle}>
          <div className="device-frame-top" aria-hidden="true" />
          <iframe title={`${profile.label} preview`} src={source} sandbox="allow-forms allow-modals allow-popups allow-scripts" />
          {showSafeArea && <div className="device-safe-area" aria-hidden="true" />}
        </div>
      </div>
      <p className="device-preview-note">
        The preview changes the iframe CSS viewport and rotation. Native sensors, OS chrome, and hardware behavior require a real emulator or device.
      </p>
    </section>
  );
}
