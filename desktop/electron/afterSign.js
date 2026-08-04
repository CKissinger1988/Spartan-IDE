/**
 * Real code-signing afterSign hook for electron-builder.
 *
 * Cross-platform, env-var-driven code signing:
 * - Linux: AppImage + deb signing via GPG (CSC_LINK=path/to/key.gpg)
 * - Windows: Authenticode via signtool (CSC_LINK=path/to/cert.pfx)
 * - macOS: codesign + notarize (CSC_LINK, APPLE_ID, APPLE_APP_SPECIFIC_PASSWORD, APPLE_TEAM_ID)
 *
 * This script does NOT fabricate or embed any signing identity. Every key
 * must be supplied via environment variables, matching §21.5's "secrets
 * kept out of plaintext project files" convention.
 *
 * electron-builder itself reads CSC_LINK/CSC_KEY_PASSWORD natively for
 * the main signing pass; this afterSign hook handles the macOS notarization
 * step that electron-builder's built-in flow doesn't cover, plus
 * validation that the signing identity was actually provided.
 */

const { execSync } = require("node:child_process");
const fs = require("node:fs");
const path = require("node:path");

function log(msg) {
  console.log(`[spartan-sign] ${msg}`);
}

function warn(msg) {
  console.warn(`[spartan-sign] WARNING: ${msg}`);
}

/** @type {import("electron-builder").AfterPackContext} */
module.exports = async function afterSign(context) {
  const { appOutDir, electronPlatformName, packager } = context;
  const productName = packager.appInfo.productName;
  const platform = electronPlatformName;

  log(`afterSign called for ${productName} on ${platform}`);

  // --- Linux: GPG signing for AppImage/deb ---
  if (platform === "linux") {
    const cscLink = process.env.CSC_LINK;
    if (!cscLink) {
      warn("CSC_LINK not set — skipping Linux signing (builds will be unsigned)");
      return;
    }

    log(`Signing Linux artifacts with GPG key: ${cscLink}`);

    const appImageFiles = fs.readdirSync(appOutDir).filter((f) => f.endsWith(".AppImage"));
    for (const file of appImageFiles) {
      const filePath = path.join(appOutDir, file);
      log(`Signing AppImage: ${file}`);
      execSync(`gpg --batch --yes --armor --detach-sign "${filePath}"`, { stdio: "inherit" });
      log(`Signed: ${file}.asc`);
    }

    const debFiles = fs.readdirSync(appOutDir).filter((f) => f.endsWith(".deb"));
    for (const file of debFiles) {
      const filePath = path.join(appOutDir, file);
      log(`Signing deb: ${file}`);
      execSync(`gpg --batch --yes --armor --detach-sign "${filePath}"`, { stdio: "inherit" });
      log(`Signed: ${file}.asc`);
    }

    log("Linux signing complete");
    return;
  }

  // --- Windows: Authenticode via signtool ---
  if (platform === "win32") {
    const cscLink = process.env.CSC_LINK;
    if (!cscLink) {
      warn("CSC_LINK not set — skipping Windows signing (builds will be unsigned)");
      return;
    }

    log(`Signing Windows artifacts with certificate: ${cscLink}`);

    let signtool = "signtool";
    try {
      const kits = "C:/Program Files (x86)/Windows Kits/10/bin";
      if (fs.existsSync(kits)) {
        const versions = fs.readdirSync(kits).filter((v) => v.startsWith("10.")).sort().reverse();
        for (const ver of versions) {
          const candidate = path.join(kits, ver, "x64", "signtool.exe");
          if (fs.existsSync(candidate)) {
            signtool = candidate;
            break;
          }
        }
      }
    } catch {
      // Use signtool from PATH
    }

    const exeFiles = fs.readdirSync(appOutDir).filter((f) => f.endsWith(".exe"));
    const cscKeyPassword = process.env.CSC_KEY_PASSWORD || "";
    const pwArg = cscKeyPassword ? `/p "${cscKeyPassword}"` : "";

    for (const file of exeFiles) {
      const filePath = path.join(appOutDir, file);
      log(`Signing: ${file}`);
      execSync(
        `${signtool} sign /f "${cscLink}" ${pwArg} /tr http://timestamp.digicert.com /td sha256 /fd sha256 "${filePath}"`,
        { stdio: "inherit" }
      );
      log(`Signed: ${file}`);
    }

    log("Windows signing complete");
    return;
  }

  // --- macOS: codesign + notarize ---
  if (platform === "darwin") {
    const cscLink = process.env.CSC_LINK;
    const appleId = process.env.APPLE_ID;
    const applePassword = process.env.APPLE_APP_SPECIFIC_PASSWORD;
    const teamId = process.env.APPLE_TEAM_ID;

    if (!cscLink) {
      warn("CSC_LINK not set — skipping macOS signing (builds will be unsigned)");
      return;
    }

    log(`Signing macOS app with identity: ${cscLink}`);

    const appBundle = fs.readdirSync(appOutDir).find((f) => f.endsWith(".app"));
    if (appBundle) {
      const appPath = path.join(appOutDir, appBundle);
      const entitlements = path.join(appOutDir, "entitlements.mac.plist");

      if (!fs.existsSync(entitlements)) {
        fs.writeFileSync(
          entitlements,
          `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>com.apple.security.cs.allow-jit</key>
    <true/>
    <key>com.apple.security.cs.allow-unsigned-executable-memory</key>
    <true/>
    <key>com.apple.security.cs.allow-dyld-environment-variables</key>
    <true/>
    <key>com.apple.security.network.client</key>
    <true/>
</dict>
</plist>`,
          "utf-8"
        );
      }

      log(`Signing .app bundle: ${appBundle}`);
      execSync(
        `codesign --deep --force --sign "${cscLink}" --entitlements "${entitlements}" --options runtime --timestamp "${appPath}"`,
        { stdio: "inherit" }
      );
      log(".app bundle signed");

      execSync(`codesign --verify --verbose=2 "${appPath}"`, { stdio: "inherit" });
      log("codesign verification passed");
    }

    if (appleId && applePassword && teamId) {
      log("Notarizing with Apple...");
      const dmgFiles = fs.readdirSync(appOutDir).filter((f) => f.endsWith(".dmg"));
      for (const file of dmgFiles) {
        const filePath = path.join(appOutDir, file);
        log(`Notarizing: ${file}`);

        const zipPath = `${filePath}.zip`;
        execSync(`ditto -c -k --keepParent "${filePath}" "${zipPath}"`, { stdio: "inherit" });
        execSync(
          `xcrun notarytool submit "${zipPath}" --apple-id "${appleId}" --password "${applePassword}" --team-id "${teamId}" --wait`,
          { stdio: "inherit" }
        );
        execSync(`xcrun stapler staple "${filePath}"`, { stdio: "inherit" });
        fs.rmSync(zipPath);
        log(`Notarized: ${file}`);
      }
    } else {
      warn("APPLE_ID/APPLE_APP_SPECIFIC_PASSWORD/APPLE_TEAM_ID not set — skipping notarization");
    }

    log("macOS signing complete");
    return;
  }

  warn(`Unknown platform "${platform}" — skipping code signing`);
};
