// Downloads the prebuilt nodewipe binary for the current platform from
// GitHub Releases, mirroring how esbuild/swc/turbo ship native binaries
// through npm. No Rust code ever gets compiled on the end user's machine.
const https = require("https");
const fs = require("fs");
const path = require("path");
const crypto = require("crypto");

const REPO = "KADHIRAVANEG/nodewipe"; 
const BIN_DIR = path.join(__dirname, "..", "bin");
const PKG = require("../package.json");

function assetNameForPlatform() {
  const platform = process.platform;
  const arch = process.arch;

  if (platform === "linux" && arch === "x64") return "nodewipe-linux-x86_64";
  if (platform === "linux" && arch === "arm64") return "nodewipe-linux-aarch64";
  if (platform === "darwin" && arch === "x64") return "nodewipe-macos-x86_64";
  if (platform === "darwin" && arch === "arm64") return "nodewipe-macos-aarch64";
  if (platform === "win32" && arch === "x64") return "nodewipe-windows-x86_64.exe";

  throw new Error(`Unsupported platform: ${platform}/${arch}. Build from source instead.`);
}

function fetchBuffer(url, redirectsLeft = 5) {
  return new Promise((resolve, reject) => {
    https
      .get(url, { headers: { "User-Agent": "nodewipe-installer" } }, (res) => {
        if ([301, 302, 307, 308].includes(res.statusCode) && res.headers.location) {
          if (redirectsLeft <= 0) return reject(new Error("Too many redirects"));
          return resolve(fetchBuffer(res.headers.location, redirectsLeft - 1));
        }
        if (res.statusCode !== 200) {
          return reject(new Error(`Download failed: HTTP ${res.statusCode} for ${url}`));
        }
        const chunks = [];
        res.on("data", (chunk) => chunks.push(chunk));
        res.on("end", () => resolve(Buffer.concat(chunks)));
      })
      .on("error", reject);
  });
}

async function main() {
  const asset = assetNameForPlatform();
  const version = PKG.version;
  const versionedUrl = `https://github.com/${REPO}/releases/download/v${version}/${asset}`;
  const sumsUrl = `https://github.com/${REPO}/releases/download/v${version}/SHA256SUMS`;
  const binName = process.platform === "win32" ? "nodewipe.exe" : "nodewipe";
  const dest = path.join(BIN_DIR, binName);

  fs.mkdirSync(BIN_DIR, { recursive: true });

  console.log(`nodewipe: downloading ${asset} (v${version}) ...`);
  const binaryData = await fetchBuffer(versionedUrl);

  // Integrity verification
  try {
    const sumsData = await fetchBuffer(sumsUrl);
    const sumsText = sumsData.toString("utf8");
    const match = sumsText.split("\n").find((line) => line.includes(asset));
    if (match) {
      const expectedHash = match.trim().split(/\s+/)[0];
      const actualHash = crypto.createHash("sha256").update(binaryData).digest("hex");
      if (expectedHash !== actualHash) {
        throw new Error(`Checksum mismatch for ${asset}! Expected: ${expectedHash}, Got: ${actualHash}`);
      }
      console.log(`nodewipe: verified SHA-256 checksum (${actualHash}).`);
    }
  } catch (err) {
    if (err.message && err.message.includes("Checksum mismatch")) {
      throw err;
    }
    // If SHA256SUMS is not yet available for an older release tag, proceed with caution
  }

  fs.writeFileSync(dest, binaryData);

  if (process.platform !== "win32") {
    fs.chmodSync(dest, 0o755);
  }

  console.log("nodewipe: installed successfully.");
}

main().catch((err) => {
  console.error(`nodewipe install failed: ${err.message}`);
  console.error("Please build from source with 'cargo install nodewipe-cli' instead.");
  process.exit(1);
});
