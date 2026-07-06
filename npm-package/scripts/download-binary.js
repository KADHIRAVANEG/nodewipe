// Downloads the prebuilt nodewipe binary for the current platform from
// GitHub Releases, mirroring how esbuild/swc/turbo ship native binaries
// through npm. No Rust code ever gets compiled on the end user's machine.
const https = require("https");
const fs = require("fs");
const path = require("path");

const REPO = "KADHIRAVANEG/nodewipe"; 
const BIN_DIR = path.join(__dirname, "..", "bin");

function assetNameForPlatform() {
  const platform = process.platform;
  const arch = process.arch;

  if (platform === "linux" && arch === "x64") return "nodewipe-linux-x86_64";
  if (platform === "darwin" && arch === "x64") return "nodewipe-macos-x86_64";
  if (platform === "darwin" && arch === "arm64") return "nodewipe-macos-aarch64";
  if (platform === "win32" && arch === "x64") return "nodewipe-windows-x86_64.exe";

  throw new Error(`Unsupported platform: ${platform}/${arch}. Build from source instead.`);
}

function download(url, dest, redirectsLeft = 5) {
  return new Promise((resolve, reject) => {
    https
      .get(url, { headers: { "User-Agent": "nodewipe-installer" } }, (res) => {
        if ([301, 302, 307, 308].includes(res.statusCode) && res.headers.location) {
          if (redirectsLeft <= 0) return reject(new Error("Too many redirects"));
          return resolve(download(res.headers.location, dest, redirectsLeft - 1));
        }
        if (res.statusCode !== 200) {
          return reject(new Error(`Download failed: HTTP ${res.statusCode} for ${url}`));
        }
        const file = fs.createWriteStream(dest);
        res.pipe(file);
        file.on("finish", () => file.close(resolve));
      })
      .on("error", reject);
  });
}

async function main() {
  const asset = assetNameForPlatform();
  const url = `https://github.com/${REPO}/releases/latest/download/${asset}`;
  const binName = process.platform === "win32" ? "nodewipe.exe" : "nodewipe";
  const dest = path.join(BIN_DIR, binName);

  fs.mkdirSync(BIN_DIR, { recursive: true });

  console.log(`nodewipe: downloading ${asset} ...`);
  await download(url, dest);

  if (process.platform !== "win32") {
    fs.chmodSync(dest, 0o755);
  }

  console.log("nodewipe: installed.");
}

main().catch((err) => {
  console.error(`nodewipe install failed: ${err.message}`);
  console.error("If no release has been published yet, build from source instead — see the main README.");
  process.exit(1);
});
